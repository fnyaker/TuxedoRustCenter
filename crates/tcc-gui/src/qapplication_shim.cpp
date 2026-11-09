// SPDX-License-Identifier: GPL-3.0-or-later
//
// Glue de build minimale : Qt Labs Platform (SystemTrayIcon/Menu) exige une
// QApplication (Qt Widgets) sous Linux. Aucune logique métier ici ; l'objet
// est créé avec des arguments factices, les options sont analysées en Rust.

#include <QApplication>
#include <QFileInfo>
#include <QGuiApplication>
#include <QIcon>
#include <QString>

extern "C" {

void *tcc_qapplication_new() {
    static int argc = 1;
    static char program[] = "tuxedo-rust-center";
    static char *argv[] = {program, nullptr};
    auto *application = new QApplication(argc, argv);

    // app_id Wayland / association au .desktop (icône launcher, dock, taskbar).
    QGuiApplication::setDesktopFileName(QStringLiteral("tuxedo-rust-center"));
    QIcon icon = QIcon::fromTheme(QStringLiteral("tuxedo-rust-center"));
    if (icon.isNull()) {
        const QString installed = QStringLiteral(
            "/usr/share/icons/hicolor/256x256/apps/tuxedo-rust-center.png");
        if (QFileInfo::exists(installed))
            icon = QIcon(installed);
    }
    if (!icon.isNull())
        application->setWindowIcon(icon);

    return static_cast<void *>(application);
}

int tcc_qapplication_exec(void *app) {
    return static_cast<QApplication *>(app)->exec();
}

void tcc_qapplication_set_identity(void *app, const char *name,
                                   const char *display, const char *version,
                                   const char *organization,
                                   const char *domain) {
    auto *application = static_cast<QApplication *>(app);
    application->setApplicationName(QString::fromUtf8(name));
    application->setApplicationDisplayName(QString::fromUtf8(display));
    application->setApplicationVersion(QString::fromUtf8(version));
    application->setOrganizationName(QString::fromUtf8(organization));
    application->setOrganizationDomain(QString::fromUtf8(domain));
}

void tcc_qapplication_delete(void *app) {
    delete static_cast<QApplication *>(app);
}

} // extern "C"
