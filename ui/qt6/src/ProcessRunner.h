#pragma once

#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>
#include <QVariantMap>
#include <qqmlintegration.h>

/// QProcess wrapper exposed to QML — runs the `pipeline-cli`
/// binary out-of-process and streams stdout / stderr as Q_SIGNAL
/// events. Streaming Docker output across a stable C ABI is
/// awkward (no closures, no callbacks), so the UI shells out to
/// the CLI for `run`. Everything else stays in-FFI.
class ProcessRunner : public QObject {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool running READ running NOTIFY runningChanged)

public:
    explicit ProcessRunner(QObject *parent = nullptr);

    bool running() const { return m_running; }

    /// `extraEnv` entries are set on the child's environment on top of the
    /// inherited one (e.g. DOCKER_HOST from Settings → Local runner).
    Q_INVOKABLE void start(const QString &program, const QStringList &args,
                           const QVariantMap &extraEnv = QVariantMap());
    Q_INVOKABLE void stop();

signals:
    void stdoutLine(const QString &line);
    void stderrLine(const QString &line);
    void started();
    void finished(int exitCode);
    void errorOccurred(const QString &message);
    void runningChanged();

private slots:
    void onStdout();
    void onStderr();
    void onFinished(int exitCode, QProcess::ExitStatus exitStatus);
    void onError(QProcess::ProcessError err);

private:
    QProcess m_proc;
    QByteArray m_stdoutBuf;
    QByteArray m_stderrBuf;
    bool m_running = false;

    void setRunning(bool v);
    void drainBuffer(QByteArray &buf, void (ProcessRunner::*emit_)(const QString &));
};
