// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! CLI TuxedoRustCenter : tout ce que fait la GUI, depuis un terminal.
//!
//! Le CLI est non privilégié. Il parle au daemon (`tccd`) pour la lecture et
//! les réglages transitoires, et aux services d'administration à la demande
//! (`tccd-admin`, `tccd-admin-hw`) pour tout ce qui est persistant ou
//! privilégié ; polkit tranche dans ces services, jamais ici.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod cli;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use serde_json::{Value, json};
use tcc_core::{Profile, ProfileCollection, Settings};

use cli::{ChargingCmd, Cli, Cmd, LightbarArg, ProfileCmd, SettingsCmd, TdpCmd, Toggle};
use tcc_client::bus::Target;
use tcc_client::client::Client;
use tcc_client::ops::resolve_profile;
use tcc_client::{jsonpath, stats};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Cmd::Status);
    if let Cmd::Completions { shell } = command {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "tcc-cli",
            &mut std::io::stdout(),
        );
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("runtime tokio")?;
    let dev_session = cli.bus.map_or_else(
        || std::env::var("TCC_DBUS_BUS").is_ok_and(|v| v.eq_ignore_ascii_case("session")),
        |bus| bus == cli::BusKind::Session,
    );
    let mut ctx = Ctx {
        client: Client::new(dev_session),
        compact: cli.json,
    };
    runtime.block_on(run(&mut ctx, command))
}

struct Ctx {
    client: Client,
    compact: bool,
}

impl Ctx {
    /// Affiche un document JSON (joli par défaut, compact avec `--json`).
    fn emit(&self, value: &Value) -> Result<()> {
        if self.compact {
            println!("{}", serde_json::to_string(value)?);
        } else {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        Ok(())
    }

    fn emit_raw(&self, raw: &str) -> Result<()> {
        let value: Value = serde_json::from_str(raw).context("réponse D-Bus JSON invalide")?;
        self.emit(&value)
    }

    /// Confirmation d'une action : texte, ou `{"ok":…}` avec `--json`.
    fn done(&self, ok: bool, message: &str) -> Result<()> {
        if self.compact {
            self.emit(&json!({"ok": ok, "message": message}))
        } else {
            println!("{message}");
            Ok(())
        }
    }

    async fn daemon_json(&mut self, target: Target, method: &str) -> Result<String> {
        self.client.json(target, method).await
    }

    async fn load_profiles(&mut self) -> Result<ProfileCollection> {
        self.client.load_profiles().await
    }

    async fn load_settings(&mut self) -> Result<Settings> {
        self.client.load_settings().await
    }

    async fn manager_bool(
        &mut self,
        method: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> Result<bool> {
        self.client.bus.call(Target::Manager, method, body).await
    }
}

/// Contenu d'un argument : littéral, `@fichier` ou `-` (stdin).
fn read_arg(arg: &str) -> Result<String> {
    use std::io::Read;
    if arg == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("lecture stdin")?;
        Ok(buffer)
    } else if let Some(path) = arg.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("lecture de {path}"))
    } else {
        Ok(arg.to_owned())
    }
}

async fn run(ctx: &mut Ctx, command: Cmd) -> Result<()> {
    match command {
        Cmd::Status => {
            let raw = ctx.daemon_json(Target::Daemon, "GetStateJSON").await?;
            ctx.emit_raw(&raw)
        }
        Cmd::Ping => {
            let ok: bool = ctx.client.bus.call(Target::Daemon, "Ping", &()).await?;
            ctx.done(
                ok,
                &format!("tccd: {}", if ok { "vivant" } else { "injoignable" }),
            )
        }
        Cmd::DaemonVersion => {
            let version: String = ctx
                .client
                .bus
                .call(Target::Daemon, "GetVersion", &())
                .await?;
            ctx.done(true, &format!("tccd {version}"))
        }
        Cmd::Profiles => profile_cmd(ctx, ProfileCmd::Dump).await,
        // Anciennes commandes, conservées pour les scripts existants.
        Cmd::SetTemp { profile } => profile_cmd(ctx, ProfileCmd::Use { profile }).await,
        Cmd::ClearTemp => profile_cmd(ctx, ProfileCmd::Clear).await,
        Cmd::Profile(sub) => profile_cmd(ctx, sub).await,
        Cmd::Settings(sub) => settings_cmd(ctx, sub).await,
        Cmd::Fans => {
            let raw = ctx.daemon_json(Target::Daemon, "GetFanDataJSON").await?;
            ctx.emit_raw(&raw)
        }
        Cmd::Hardware => {
            let raw = ctx
                .daemon_json(Target::Manager, "GetHardwareSnapshotJSON")
                .await?;
            ctx.emit_raw(&raw)
        }
        Cmd::Timer => {
            let raw = ctx
                .daemon_json(Target::Manager, "GetShutdownTimerJSON")
                .await?;
            ctx.emit_raw(&raw)
        }
        Cmd::Sensors { action } => sensors_cmd(ctx, action).await,
        Cmd::Stats { no_sensors } => stats_cmd(ctx, no_sensors).await,
        Cmd::Charging(sub) => charging_cmd(ctx, sub).await,
        Cmd::Keyboard { states } => {
            let payload = read_arg(&states)?;
            serde_json::from_str::<Value>(&payload).context("JSON des zones clavier invalide")?;
            let ok = ctx
                .manager_bool("SetKeyboardStatesJSON", &payload.as_str())
                .await?;
            ctx.done(ok, &format!("éclairage clavier appliqué: {ok}"))
        }
        Cmd::Lightbar { state } => {
            let payload = match state {
                LightbarArg::Off => "null".to_owned(),
                LightbarArg::Json(arg) => {
                    let payload = read_arg(&arg)?;
                    serde_json::from_str::<Value>(&payload).context("JSON lightbar invalide")?;
                    payload
                }
            };
            let ok = ctx
                .manager_bool("SetLightbarStatesJSON", &payload.as_str())
                .await?;
            ctx.done(ok, &format!("lightbar appliquée: {ok}"))
        }
        Cmd::FnLock { state } => {
            let enabled = state == Toggle::On;
            let ok = ctx.manager_bool("SetFnLockStatus", &enabled).await?;
            ctx.done(
                ok,
                &format!("Fn Lock {}: {ok}", if enabled { "on" } else { "off" }),
            )
        }
        Cmd::Tdp(sub) => tdp_cmd(ctx, sub).await,
        Cmd::Odm { name, r#override } => {
            let method = if r#override {
                "SetOdmProfileOverride"
            } else {
                "SetOdmProfile"
            };
            let ok: bool = ctx
                .client
                .bus
                .call(Target::AdminHw, method, &name.as_str())
                .await?;
            ctx.done(ok, &format!("profil ODM '{name}': {ok}"))
        }
        Cmd::Ctgp { offset } => {
            let ok: bool = ctx
                .client
                .bus
                .call(Target::AdminHw, "SetNvidiaCtgpOffset", &offset)
                .await?;
            ctx.done(ok, &format!("offset cTGP {offset} W: {ok}"))
        }
        Cmd::Display => {
            let raw: String = ctx
                .client
                .bus
                .call(Target::Session, "GetDisplayInfoJSON", &())
                .await?;
            ctx.emit_raw(&raw)
        }
        Cmd::Uvc { device, key, value } => {
            let ok: bool = ctx
                .client
                .bus
                .call(
                    Target::Session,
                    "SetUvcControl",
                    &(device.as_str(), key.as_str(), value),
                )
                .await?;
            ctx.done(
                ok,
                &format!(
                    "contrôle UVC '{key}'={value}: {}",
                    if ok {
                        "écrit"
                    } else {
                        "déjà à cette valeur"
                    }
                ),
            )
        }
        Cmd::Admin => {
            let ping: bool = ctx.client.bus.call(Target::Admin, "Ping", &()).await?;
            let version: String = ctx
                .client
                .bus
                .call(Target::Admin, "GetVersion", &())
                .await?;
            let contract: u32 = ctx
                .client
                .bus
                .call(Target::Admin, "GetContractVersion", &())
                .await?;
            if ctx.compact {
                ctx.emit(&json!({"ping": ping, "version": version, "contract": contract}))
            } else {
                println!("admin: ping={ping}, version={version}, contrat={contract}");
                Ok(())
            }
        }
        Cmd::Reload => {
            let ok: bool = ctx
                .client
                .bus
                .call(Target::Admin, "ReloadConfig", &())
                .await?;
            ctx.done(ok, &format!("configuration rechargée: {ok}"))
        }
        Cmd::Completions { .. } => Ok(()),
    }
}

async fn profile_cmd(ctx: &mut Ctx, sub: ProfileCmd) -> Result<()> {
    match sub {
        ProfileCmd::Dump => {
            let raw = ctx.daemon_json(Target::Daemon, "GetProfilesJSON").await?;
            ctx.emit_raw(&raw)
        }
        ProfileCmd::List => {
            let collection = ctx.load_profiles().await?;
            let state: Value =
                serde_json::from_str(&ctx.daemon_json(Target::Daemon, "GetStateJSON").await?)
                    .unwrap_or(Value::Null);
            let active = state["activeProfile"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if ctx.compact {
                let list: Vec<Value> = collection
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id.as_str(),
                            "name": p.name,
                            "description": p.description,
                            "isPreset": p.is_preset(),
                            "active": p.id.as_str() == active,
                        })
                    })
                    .collect();
                return ctx.emit(&Value::Array(list));
            }
            for p in collection.iter() {
                let mark = if p.id.as_str() == active { '*' } else { ' ' };
                println!("{mark} {:<28} {}", p.id.as_str(), p.name);
            }
            Ok(())
        }
        ProfileCmd::Show { profile } => {
            let collection = ctx.load_profiles().await?;
            let found = resolve_profile(&collection, &profile)?;
            ctx.emit(&serde_json::to_value(found)?)
        }
        ProfileCmd::Active => {
            let raw = ctx
                .daemon_json(Target::Daemon, "GetActiveProfileJSON")
                .await?;
            ctx.emit_raw(&raw)
        }
        ProfileCmd::Use { profile } => {
            let id = ctx.client.use_profile(&profile).await?;
            ctx.done(true, &format!("profil temporaire '{id}': true"))
        }
        ProfileCmd::Clear => {
            ctx.client.clear_temp().await?;
            ctx.done(true, "override temporaire supprimé: true")
        }
        ProfileCmd::Create { name } => {
            let id = ctx.client.create_profile(name).await?;
            ctx.done(true, &format!("profil créé: {id}"))
        }
        ProfileCmd::Copy { from, name } => {
            let id = ctx.client.copy_profile(&from, name).await?;
            ctx.done(true, &format!("profil copié: {id}"))
        }
        ProfileCmd::Delete { profile } => {
            let id = ctx.client.delete_profile(&profile).await?;
            ctx.done(true, &format!("profil supprimé: {id}"))
        }
        ProfileCmd::Rename { profile, name } => {
            ctx.client.rename_profile(&profile, name).await?;
            ctx.done(true, "profil renommé")
        }
        ProfileCmd::Set {
            profile,
            key,
            value,
        } => {
            ctx.client.set_profile_field(&profile, &key, &value).await?;
            ctx.done(true, &format!("{key} mis à jour"))
        }
        ProfileCmd::Get { profile, key } => {
            let collection = ctx.load_profiles().await?;
            let document = serde_json::to_value(resolve_profile(&collection, &profile)?)?;
            let value = jsonpath::get_path(&document, &key)
                .with_context(|| format!("clé absente : {key}"))?;
            ctx.emit(value)
        }
        ProfileCmd::Export { profile, file } => {
            let collection = ctx.load_profiles().await?;
            let text = serde_json::to_string_pretty(resolve_profile(&collection, &profile)?)?;
            match file {
                Some(path) => {
                    std::fs::write(&path, format!("{text}\n"))
                        .with_context(|| format!("écriture de {path}"))?;
                    ctx.done(true, &format!("profil exporté vers {path}"))
                }
                None => {
                    println!("{text}");
                    Ok(())
                }
            }
        }
        ProfileCmd::Import { file } => {
            let text = read_arg(&if file == "-" {
                file
            } else {
                format!("@{file}")
            })?;
            let profile: Profile =
                serde_json::from_str(&text).context("profil importé invalide")?;
            let id = ctx.client.import_profile(profile).await?;
            ctx.done(true, &format!("profil importé: {id}"))
        }
    }
}

async fn settings_cmd(ctx: &mut Ctx, sub: SettingsCmd) -> Result<()> {
    match sub {
        SettingsCmd::Show => {
            let raw = ctx.daemon_json(Target::Daemon, "GetSettingsJSON").await?;
            ctx.emit_raw(&raw)
        }
        SettingsCmd::Get { key } => {
            let settings = ctx.load_settings().await?;
            let document = serde_json::to_value(&settings)?;
            let value = jsonpath::get_path(&document, &key)
                .with_context(|| format!("clé absente : {key}"))?;
            ctx.emit(value)
        }
        SettingsCmd::Set { key, value } => {
            ctx.client.set_setting(&key, &value).await?;
            ctx.done(true, &format!("{key} mis à jour"))
        }
    }
}

async fn sensors_cmd(ctx: &mut Ctx, action: Option<Toggle>) -> Result<()> {
    match action {
        None | Some(Toggle::Status) => {
            let on: bool = ctx
                .client
                .bus
                .call(Target::Manager, "GetSensorDataCollectionStatus", &())
                .await?;
            ctx.done(
                on,
                &format!(
                    "collecte capteurs: {}",
                    if on { "active" } else { "inactive" }
                ),
            )
        }
        Some(toggle) => {
            let enabled = toggle == Toggle::On;
            ctx.manager_bool("SetSensorDataCollectionStatus", &enabled)
                .await?;
            ctx.done(
                true,
                &format!(
                    "collecte capteurs: {}",
                    if enabled {
                        "activée (bail 10 s)"
                    } else {
                        "désactivée"
                    }
                ),
            )
        }
    }
}

async fn stats_cmd(ctx: &mut Ctx, no_sensors: bool) -> Result<()> {
    let root = std::env::var("TCC_POWER_SUPPLY_ROOT")
        .unwrap_or_else(|_| "/sys/class/power_supply".to_owned());
    // La mesure CPU bloque ~200 ms : hors du runtime async.
    let usage =
        tokio::task::spawn_blocking(|| stats::cpu_usage(std::time::Duration::from_millis(200)))
            .await
            .ok()
            .flatten();
    let battery = stats::battery(std::path::Path::new(&root));

    let mut daemon_error: Option<String> = None;
    let state = match ctx.daemon_json(Target::Daemon, "GetStateJSON").await {
        Ok(raw) => serde_json::from_str::<Value>(&raw).ok(),
        Err(error) => {
            daemon_error = Some(format!("{error:#}"));
            None
        }
    };

    let mut cpu_power: Option<f64> = None;
    let mut cpu_temp: Option<f64> = None;
    if state.is_some() && !no_sensors {
        if let Ok(raw) = ctx
            .daemon_json(Target::Manager, "GetHardwareSnapshotJSON")
            .await
        {
            let snapshot: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            let sensors = &snapshot["sensors"];
            if sensors["enabled"].as_bool() == Some(true) {
                cpu_power = sensors["cpu"]["packagePowerW"].as_f64();
                cpu_temp = sensors["cpu"]["temperatureC"].as_f64();
            } else {
                // Bail de 10 s : renouvelé par les lectures suivantes ; la
                // première mesure de puissance arrive donc au 2ᵉ appel.
                let _ = ctx
                    .manager_bool("SetSensorDataCollectionStatus", &true)
                    .await;
            }
        }
    }

    let active = state.as_ref().map(|s| &s["activeProfile"]);
    let report = json!({
        "daemon": state.is_some(),
        "daemonError": daemon_error,
        "profile": active.map(|a| json!({"id": a["id"], "name": a["name"], "source": a["source"]})),
        "ppd": state.as_ref().map(|s| s["ppdProfile"].clone()),
        "powerSource": state.as_ref().map(|s| s["powerSource"].clone()),
        "manualOverride": state.as_ref().map(|s| s["manualOverride"].clone()),
        "cpu": {"usagePercent": usage, "packagePowerW": cpu_power, "temperatureC": cpu_temp},
        "battery": battery,
    });
    if ctx.compact {
        return ctx.emit(&report);
    }
    let fmt = |value: Option<f64>, unit: &str| {
        value.map_or_else(|| "—".to_owned(), |v| format!("{v:.1} {unit}"))
    };
    match &state {
        Some(_) => println!(
            "profil   : {} ({})",
            report["profile"]["name"].as_str().unwrap_or("?"),
            report["profile"]["id"].as_str().unwrap_or("?")
        ),
        None => println!("profil   : daemon injoignable"),
    }
    println!(
        "état     : {} / ppd {}",
        report["powerSource"].as_str().unwrap_or("?"),
        report["ppd"].as_str().unwrap_or("—")
    );
    println!(
        "cpu      : {} · {} · {}",
        fmt(usage, "%"),
        fmt(cpu_power, "W"),
        fmt(cpu_temp, "°C")
    );
    if report["battery"]["present"] == true {
        println!(
            "batterie : {} % · {} · {}",
            report["battery"]["percent"],
            report["battery"]["status"].as_str().unwrap_or("?"),
            fmt(report["battery"]["powerW"].as_f64(), "W")
        );
    } else {
        println!("batterie : absente");
    }
    Ok(())
}

async fn charging_cmd(ctx: &mut Ctx, sub: ChargingCmd) -> Result<()> {
    let (method, ok) = match sub {
        ChargingCmd::Show => {
            let raw = ctx
                .daemon_json(Target::Manager, "GetHardwareSnapshotJSON")
                .await?;
            let snapshot: Value = serde_json::from_str(&raw).context("snapshot invalide")?;
            return ctx.emit(&snapshot["charging"]);
        }
        ChargingCmd::Profile { name } => (
            "SetChargingProfile",
            ctx.manager_bool("SetChargingProfile", &name.as_str())
                .await?,
        ),
        ChargingCmd::Priority { name } => (
            "SetChargingPriority",
            ctx.manager_bool("SetChargingPriority", &name.as_str())
                .await?,
        ),
        ChargingCmd::Start { percent } => (
            "SetChargeStartThreshold",
            ctx.manager_bool("SetChargeStartThreshold", &i32::from(percent))
                .await?,
        ),
        ChargingCmd::End { percent } => (
            "SetChargeEndThreshold",
            ctx.manager_bool("SetChargeEndThreshold", &i32::from(percent))
                .await?,
        ),
        ChargingCmd::Type { name } => (
            "SetChargeType",
            ctx.manager_bool("SetChargeType", &name.as_str()).await?,
        ),
    };
    ctx.done(ok, &format!("{method}: {ok}"))
}

async fn tdp_cmd(ctx: &mut Ctx, sub: TdpCmd) -> Result<()> {
    match sub {
        TdpCmd::Get => {
            let values: Vec<i32> = ctx
                .client
                .bus
                .call(Target::AdminHw, "GetTdpValues", &())
                .await?;
            ctx.emit(&json!(values))
        }
        TdpCmd::Set { watts } => {
            let ok: bool = ctx
                .client
                .bus
                .call(Target::AdminHw, "SetTdpValues", &watts)
                .await?;
            ctx.done(ok, &format!("TDP {watts:?} W: {ok}"))
        }
        TdpCmd::Reset => {
            let ok: bool = ctx
                .client
                .bus
                .call(Target::AdminHw, "SetTdpValues", &Vec::<i32>::new())
                .await?;
            ctx.done(ok, &format!("TDP remis aux maxima firmware: {ok}"))
        }
    }
}
