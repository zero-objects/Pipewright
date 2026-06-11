#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>

int main(int argc, char *argv[]) {
    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("Pipewright"));
    app.setOrganizationName(QStringLiteral("Zero Objects"));
    app.setOrganizationDomain(QStringLiteral("zero-principle.org"));

    QQuickStyle::setStyle(QStringLiteral("Fusion"));

    QQmlApplicationEngine engine;
    // Headless smoke mode (PIPEWRIGHT_SMOKE=1): Main.qml self-checks the FFI
    // bridge + a render path and Qt.exit()s with a pass/fail code, so CI can
    // run the real UI offscreen as an end-to-end smoke test. The optional
    // PIPEWRIGHT_SMOKE_FILE is loaded first.
    engine.rootContext()->setContextProperty(
        QStringLiteral("smokeMode"), qEnvironmentVariableIsSet("PIPEWRIGHT_SMOKE"));
    engine.rootContext()->setContextProperty(
        QStringLiteral("smokeFile"), QString::fromUtf8(qgetenv("PIPEWRIGHT_SMOKE_FILE")));
    // Screenshot mode (PIPEWRIGHT_SHOT=1): load PIPEWRIGHT_SMOKE_FILE and jump
    // to the DAG tab, then leave the window up for a screen capture. No exit.
    engine.rootContext()->setContextProperty(
        QStringLiteral("shotMode"), qEnvironmentVariableIsSet("PIPEWRIGHT_SHOT"));

    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreationFailed,
        &app, []() { QCoreApplication::exit(-1); },
        Qt::QueuedConnection);
    engine.loadFromModule(QStringLiteral("Pipewright"),
                          QStringLiteral("Main"));

    return app.exec();
}
