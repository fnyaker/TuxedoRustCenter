%global debug_package %{nil}

# tccd-admin (phase 1) : installé PAR DÉFAUT (activation D-Bus à la demande).
# Désactiver avec : rpmbuild --without admin ...
%bcond_without admin

# tccd-admin-hw (phase 2, AD-12 option B2) : TDP/ODM/cTGP. Préparé, NON
# installé par défaut. Activer avec
#   rpmbuild --with admin_hw ...
# uniquement quand tccd-admin-hw existe et est validé matériellement.
# Indépendant de --with admin : ce dernier n'installe PAS Admin.Hw.
%bcond_with admin_hw

# Interface gpui (style Omarchy), workspace séparé (rustc >= 1.92). Activer avec
#   rpmbuild --with gpui ...
%bcond_with gpui

Name:           tuxedo-rust-center
Version:        0.1.4
Release:        1%{?dist}
Summary:        Native Qt 6 TUXEDO Control Center rewrite

License:        GPL-3.0-or-later
URL:            https://github.com/fnyaker/TuxedoRustCenter
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  make
BuildRequires:  qt6-qtbase-devel
BuildRequires:  qt6-qtdeclarative-devel
BuildRequires:  qt6-qtsvg-devel
BuildRequires:  systemd-rpm-macros
BuildRequires:  libxml2
BuildRequires:  desktop-file-utils
BuildRequires:  appstream

Requires:       qt6-qtbase
Requires:       qt6-qtbase-gui
Requires:       qt6-qtdeclarative
Requires:       qt6-qt5compat
Requires:       qt6-qtsvg
Requires:       hicolor-icon-theme
Requires:       systemd
Requires(post): systemd
Requires(preun): systemd
Requires(postun): systemd

# Dépendances optionnelles : le logiciel fonctionne partiellement sans elles.
Recommends:     tuxedo-drivers
Recommends:     power-profiles-daemon
Recommends:     upower
Recommends:     polkit
Suggests:       tuxedo-tomte
Suggests:       tuxedo-systeminfos
Suggests:       kscreen
Suggests:       switcheroo-control

# Cohabitation impossible avec TCC original : même unité tccd.service, même
# nom D-Bus com.tuxedocomputers.tccd, mêmes écritures sysfs.
Conflicts:      tuxedo-control-center
Obsoletes:      tuxedo-control-center < %{version}-%{release}
Provides:       tuxedo-control-center = %{version}-%{release}
Conflicts:      tuxedofancontrol
Obsoletes:      tuxedofancontrol <= 0.1.9

%description
TuxedoRustCenter is a 100% Rust rewrite of the TUXEDO Control Center, with a
native Qt 6 GUI, a privileged Rust daemon and an unprivileged session agent.

Features include hardware profiles for CPU limits, fans, display, webcam and
charging; automatic profile switching from power-profiles-daemon and the
AC/battery state; internal display refresh rate per profile on X11 and Wayland;
and a system dashboard with temperatures, power and fan speeds.

The package installs no file under /opt and requires no Node/Python
runtime.

%prep
%setup -q -n %{name}-%{version}

%build
%set_build_flags
export SOURCE_DATE_EPOCH=%{?source_date_epoch:%{source_date_epoch}}%{!?source_date_epoch:1793923200}
cargo build --release --locked
%if %{with gpui}
cargo build --release --locked --manifest-path crates/tcc-gpui/Cargo.toml
%endif

%install
rm -rf %{buildroot}
%if %{with admin}
export TCC_WITH_ADMIN=1
%else
export TCC_WITH_ADMIN=0
%endif
%if %{with admin_hw}
export TCC_WITH_ADMIN_HW=1
%endif
%if %{with gpui}
export TCC_WITH_GPUI=1
%endif
scripts/release/install-tree.sh %{buildroot}

%check
# Validation sans réseau. Les outils absents sont ignorés, les échecs non.
TCC_VALIDATE_STRICT=0 scripts/release/validate.sh --tree %{buildroot}

%files
%license /usr/share/licenses/tuxedo-rust-center/LICENSE
%doc /usr/share/doc/tuxedo-rust-center/copyright
%doc /usr/share/doc/tuxedo-rust-center/LICENSE
%doc /usr/share/doc/tuxedo-rust-center/changelog.gz
%doc /usr/share/doc/tuxedo-rust-center/PLATFORM_DESIGN.md
%doc /usr/share/doc/tuxedo-rust-center/TCC_USER_DESIGN.md
%doc /usr/share/doc/tuxedo-rust-center/DBUS_API.md
%doc /usr/share/doc/tuxedo-rust-center/MIGRATION.md
%doc /usr/share/doc/tuxedo-rust-center/SECURITY.md
%doc /usr/share/doc/tuxedo-rust-center/ATTRIBUTION.md
%config(noreplace) /etc/tuxedo-rust-center/README
%dir /etc/tuxedo-rust-center
%dir /var/lib/tuxedo-rust-center
%dir /usr/share/tuxedo-rust-center
%{_bindir}/tuxedo-rust-center
%{_bindir}/tcc-cli
%dir %{_libexecdir}/tuxedo-rust-center
%{_libexecdir}/tuxedo-rust-center/tccd
%{_libexecdir}/tuxedo-rust-center/tccd-user
%{_libexecdir}/tuxedo-rust-center/tcc-gui-qt
%if %{with gpui}
%{_libexecdir}/tuxedo-rust-center/tcc-gui-gpui
%endif
%if %{with admin}
%{_libexecdir}/tuxedo-rust-center/tccd-admin
%{_unitdir}/tccd-admin.service
%{_datadir}/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.service
%endif
%if %{with admin_hw}
%{_libexecdir}/tuxedo-rust-center/tccd-admin-hw
%{_unitdir}/tccd-admin-hw.service
%{_datadir}/dbus-1/system-services/com.tuxedocomputers.tccd.Admin.Hw.service
%endif
%{_unitdir}/tccd.service
%{_userunitdir}/tccd-user.service
%dir %{_userunitdir}/graphical-session.target.wants
%{_userunitdir}/graphical-session.target.wants/tccd-user.service
%{_datadir}/dbus-1/system.d/com.tuxedocomputers.tccd.conf
%{_datadir}/polkit-1/actions/com.tuxedocomputers.tccd.policy
%{_udevrulesdir}/99-tuxedo-rust-center-webcam.rules
%{_tmpfilesdir}/tuxedo-rust-center.conf
%{_datadir}/applications/tuxedo-rust-center.desktop
%{_datadir}/icons/hicolor/*/apps/tuxedo-rust-center.*
%{_datadir}/metainfo/com.tuxedocomputers.TuxedoRustCenter.metainfo.xml
%{_datadir}/tuxedo-rust-center/qml
%{_datadir}/tuxedo-rust-center/assets
%{_datadir}/tuxedo-rust-center/omarchy-plugin

%post
# Pas de %systemd_post inconditionnel : une installation hors paquet de TCC
# (/opt) ne doit pas être activée au prochain démarrage.
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload >/dev/null 2>&1 || :
    # Activation uniquement à l'installation initiale (pas de réactivation
    # d'un service désactivé par l'administrateur lors d'une mise à jour).
    if [ "$1" -eq 1 ] && [ ! -d /opt/tuxedo-control-center ] && [ ! -e /usr/bin/tuxedo-control-center ]; then
        systemctl preset tccd.service >/dev/null 2>&1 || :
    fi
fi
# Lien d'activation laissé par l'ancien TCC : l'unité tccd-sleep n'existe plus.
if [ -L /etc/systemd/system/sleep.target.wants/tccd-sleep.service ] && [ ! -e /usr/lib/systemd/system/tccd-sleep.service ]; then
    rm -f /etc/systemd/system/sleep.target.wants/tccd-sleep.service
fi

# Avertissement de cohabitation avec une installation hors paquet de TCC.
if [ -d /opt/tuxedo-control-center ] || [ -e /usr/bin/tuxedo-control-center ]; then
    echo "AVERTISSEMENT: une installation de TUXEDO Control Center a été détectée" >&2
    echo "dans /opt. Arrêter l'ancien service avant d'activer tccd :" >&2
    echo "  systemctl disable --now tccd" >&2
    echo "  systemctl enable --now tccd.service" >&2
    echo "Voir /usr/share/doc/tuxedo-rust-center/MIGRATION.md" >&2
fi

%preun
# tccd-admin / tccd-admin-hw ne sont présents qu'avec --with admin /
# --with admin_hw ; chaque stop est un no-op sinon.
if [ -d /run/systemd/system ]; then
    systemctl stop tccd-admin-hw.service >/dev/null 2>&1 || :
    systemctl stop tccd-admin.service >/dev/null 2>&1 || :
fi
%systemd_preun tccd.service

%postun
%systemd_postun_with_restart tccd.service

* Tue Nov 10 2026 TuxedoRustCenter contributors <maintainers@tuxedorustcenter.invalid> - 0.1.4-1
- tcc-cli stats : la puissance CPU du paquet est de nouveau renvoyée (le
  plugin Omarchy l'affiche).

%changelog
* Tue Nov 10 2026 TuxedoRustCenter contributors <maintainers@tuxedorustcenter.invalid> - 0.1.3-1
- Interface gpui complète (dashboard avec stats GPU, profils, éditeur avec
  courbe de ventilation éditable, outils, réglages) ; tccd-admin accepte une
  sauvegarde quand d'anciens presets sont présents sur disque.

* Tue Nov 10 2026 TuxedoRustCenter contributors <maintainers@tuxedorustcenter.invalid> - 0.1.2-1
- CLI complète (tcc-cli), lanceur d'interface auto/qt6/gpui, plugin de barre
  Omarchy, tray monochrome avec recherche de profils ; interface gpui avec
  --with gpui.

* Tue Nov 10 2026 TuxedoRustCenter contributors <maintainers@tuxedorustcenter.invalid> - 0.1.1-1
- tccd-admin (phase 1) installé par défaut ; --without admin pour l'exclure.

* Fri Nov 06 2026 TuxedoRustCenter contributors <maintainers@tuxedorustcenter.invalid> - 0.1.0-1
- Premier paquet installable (jalon 1/2) : tccd, tccd-user, GUI Qt 6 et CLI.
- Units systemd système et utilisateur, politique D-Bus, polkit, udev sans
  RUN+=, tmpfiles, desktop entry, icônes hicolor, métadonnées AppStream.
- FHS strict, aucun /opt, aucun runtime Node/Python, aucun setuid.
- État mutable dans /var/lib/tuxedo-rust-center (D2), /etc en lecture seule
  pour le daemon.
- Conflit explicite avec tuxedo-control-center (unité tccd.service et nom
  D-Bus partagés).
- tccd-admin non installé : squelette présent, service non fonctionnel (D1).
  Packaging phase 1 préparé (Admin1/Control1, polkit) avec --with admin ;
  par défaut, rien n'est installé.
- tccd-admin-hw (phase 2, AD-12 B2 : TDP/ODM/cTGP) préparé mais non installé.
  Unité durcie, activation D-Bus et policy membre par membre derrière
  --with admin_hw ; --with admin seul ne l'installe pas. Binaire
  tccd-admin-hw livré ; validation fonctionnelle/matérielle en cours.
- Dépendances QML complétées : Qt.labs.platform, QtQuick.Dialogs,
  Qt5Compat.GraphicalEffects, QtQml.Models et Qt6Widgets/QApplication.
