// SPDX-License-Identifier: GPL-3.0-or-later
//! IPC bas niveau vers les compositeurs Wayland.
//!
//! - Hyprland : socket sans préfixe, réponse jusqu'à EOF.
//! - Sway : protocole i3-ipc (magic `i3-ipc`, longueur et type en boutisme
//!   natif).
//!
//! Toutes les connexions passent par [`crate::security::connect_session_socket`]
//! (propriétaire, type, permissions, credentials du pair, timeouts).

use std::io::{Read, Write};
use std::net::Shutdown;
use std::path::Path;

use crate::error::{Result, UserError};
use crate::security::{MAX_IPC_REPLY_BYTES, connect_session_socket};

/// Magic du protocole i3-ipc.
pub const I3_IPC_MAGIC: &[u8; 6] = b"i3-ipc";

/// Types de messages i3-ipc utilisés.
pub mod sway_message {
    /// Exécute une commande (`RUN_COMMAND`).
    pub const RUN_COMMAND: u32 = 0;
    /// Liste les sorties (`GET_OUTPUTS`).
    pub const GET_OUTPUTS: u32 = 3;
}

/// Encode un message i3-ipc complet (en-tête + payload).
pub fn encode_sway_message(message_type: u32, payload: &[u8]) -> Result<Vec<u8>> {
    let length = u32::try_from(payload.len())
        .map_err(|_| UserError::InvalidArgument("payload i3-ipc trop grand".to_owned()))?;
    let mut buffer = Vec::with_capacity(14 + payload.len());
    buffer.extend_from_slice(I3_IPC_MAGIC);
    buffer.extend_from_slice(&length.to_ne_bytes());
    buffer.extend_from_slice(&message_type.to_ne_bytes());
    buffer.extend_from_slice(payload);
    Ok(buffer)
}

/// Envoie une requête à Hyprland et retourne la réponse texte.
pub fn hyprland_request(path: &Path, uid: u32, command: &str) -> Result<String> {
    if command.is_empty()
        || command.len() > 4096
        || command
            .bytes()
            .any(|byte| byte == 0 || byte == b'\n' || byte == b'\r')
    {
        return Err(UserError::InvalidArgument(
            "commande Hyprland invalide".to_owned(),
        ));
    }
    let mut stream = connect_session_socket(path, uid)?;
    stream.write_all(command.as_bytes())?;
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    read_limited(&mut stream, MAX_IPC_REPLY_BYTES)
}

/// Lit une réponse bornée et la décode en UTF-8.
pub fn read_limited(reader: &mut impl Read, limit: usize) -> Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        if buffer.len() + count > limit {
            return Err(UserError::Ipc {
                protocol: "unix",
                message: format!("réponse supérieure à {limit} octets"),
            });
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(buffer).map_err(|_| UserError::Ipc {
        protocol: "unix",
        message: "réponse non UTF-8".to_owned(),
    })
}

/// Envoie une requête i3-ipc (Sway) et retourne le payload de réponse.
pub fn sway_request(
    path: &Path,
    uid: u32,
    message_type: u32,
    payload: &[u8],
) -> Result<(u32, Vec<u8>)> {
    let mut stream = connect_session_socket(path, uid)?;
    let message = encode_sway_message(message_type, payload)?;
    stream.write_all(&message)?;
    stream.flush()?;

    let mut header = [0u8; 14];
    read_exact(&mut stream, &mut header)?;
    if &header[..6] != I3_IPC_MAGIC {
        return Err(UserError::Ipc {
            protocol: "sway",
            message: "magic i3-ipc invalide".to_owned(),
        });
    }
    let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]);
    let reply_type = u32::from_ne_bytes([header[10], header[11], header[12], header[13]]);
    let length = usize::try_from(length).map_err(|_| UserError::Ipc {
        protocol: "sway",
        message: "longueur de réponse invalide".to_owned(),
    })?;
    if length > MAX_IPC_REPLY_BYTES {
        return Err(UserError::Ipc {
            protocol: "sway",
            message: format!("réponse de {length} octets refusée"),
        });
    }
    let mut body = vec![0u8; length];
    read_exact(&mut stream, &mut body)?;
    Ok((reply_type, body))
}

fn read_exact(reader: &mut impl Read, buffer: &mut [u8]) -> Result<()> {
    reader.read_exact(buffer).map_err(|err| UserError::Ipc {
        protocol: "sway",
        message: format!("lecture de l'en-tête/réponse : {err}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sway_header_is_i3_ipc() {
        let message = encode_sway_message(sway_message::GET_OUTPUTS, b"").unwrap();
        assert_eq!(&message[..6], b"i3-ipc");
        assert_eq!(
            u32::from_ne_bytes([message[6], message[7], message[8], message[9]]),
            0
        );
        assert_eq!(
            u32::from_ne_bytes([message[10], message[11], message[12], message[13]]),
            sway_message::GET_OUTPUTS
        );
        assert_eq!(message.len(), 14);
    }

    #[test]
    fn sway_header_encodes_payload_length() {
        let message = encode_sway_message(sway_message::RUN_COMMAND, b"output eDP-1").unwrap();
        assert_eq!(message.len(), 14 + 12);
        assert_eq!(
            u32::from_ne_bytes([message[6], message[7], message[8], message[9]]),
            12
        );
    }

    #[test]
    fn read_limited_rejects_oversized() {
        let data = vec![b'a'; 100];
        let mut cursor = std::io::Cursor::new(data);
        assert!(read_limited(&mut cursor, 10).is_err());
    }

    #[test]
    fn read_limited_reads_small_response() {
        let data = b"ok".to_vec();
        let mut cursor = std::io::Cursor::new(data);
        assert_eq!(read_limited(&mut cursor, 10).unwrap(), "ok");
    }

    #[test]
    fn hyprland_rejects_multiline_command() {
        let err = hyprland_request(
            std::path::Path::new("/nonexistent"),
            1000,
            "keyword monitor x\nreload",
        )
        .unwrap_err();
        assert!(matches!(err, UserError::InvalidArgument(_)));
    }
}
