#include "ProcessRunner.h"

ProcessRunner::ProcessRunner(QObject *parent) : QObject(parent) {
    connect(&m_proc, &QProcess::readyReadStandardOutput, this, &ProcessRunner::onStdout);
    connect(&m_proc, &QProcess::readyReadStandardError, this, &ProcessRunner::onStderr);
    connect(&m_proc, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished),
            this, &ProcessRunner::onFinished);
    connect(&m_proc, &QProcess::errorOccurred, this, &ProcessRunner::onError);
}

void ProcessRunner::start(const QString &program, const QStringList &args,
                          const QVariantMap &extraEnv) {
    if (m_running) {
        emit errorOccurred(QStringLiteral("a process is already running"));
        return;
    }
    m_stdoutBuf.clear();
    m_stderrBuf.clear();
    m_proc.setProgram(program);
    m_proc.setArguments(args);
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    for (auto it = extraEnv.constBegin(); it != extraEnv.constEnd(); ++it) {
        const QString value = it.value().toString();
        if (!value.isEmpty()) {
            env.insert(it.key(), value);
        }
    }
    m_proc.setProcessEnvironment(env);
    m_proc.start();
    if (m_proc.waitForStarted(3000)) {
        setRunning(true);
        emit started();
    } else {
        emit errorOccurred(QStringLiteral("failed to start %1: %2")
                               .arg(program, m_proc.errorString()));
    }
}

void ProcessRunner::stop() {
    if (!m_running) return;
    m_proc.terminate();
    if (!m_proc.waitForFinished(2000)) {
        m_proc.kill();
    }
}

void ProcessRunner::onStdout() {
    m_stdoutBuf.append(m_proc.readAllStandardOutput());
    drainBuffer(m_stdoutBuf, &ProcessRunner::stdoutLine);
}

void ProcessRunner::onStderr() {
    m_stderrBuf.append(m_proc.readAllStandardError());
    drainBuffer(m_stderrBuf, &ProcessRunner::stderrLine);
}

void ProcessRunner::onFinished(int exitCode, QProcess::ExitStatus) {
    if (!m_stdoutBuf.isEmpty()) {
        emit stdoutLine(QString::fromUtf8(m_stdoutBuf));
        m_stdoutBuf.clear();
    }
    if (!m_stderrBuf.isEmpty()) {
        emit stderrLine(QString::fromUtf8(m_stderrBuf));
        m_stderrBuf.clear();
    }
    setRunning(false);
    emit finished(exitCode);
}

void ProcessRunner::onError(QProcess::ProcessError) {
    if (m_running) {
        setRunning(false);
    }
    emit errorOccurred(m_proc.errorString());
}

void ProcessRunner::setRunning(bool v) {
    if (m_running == v) return;
    m_running = v;
    emit runningChanged();
}

void ProcessRunner::drainBuffer(QByteArray &buf,
                                void (ProcessRunner::*emit_)(const QString &)) {
    while (true) {
        const int nl = buf.indexOf('\n');
        if (nl < 0) break;
        const QString line = QString::fromUtf8(buf.left(nl));
        buf.remove(0, nl + 1);
        (this->*emit_)(line);
    }
}
