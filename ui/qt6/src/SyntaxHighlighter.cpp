#include "SyntaxHighlighter.h"

#include <QColor>
#include <QTextDocument>

namespace {

// Palette tuned for a light background; deliberately muted so the
// emphasis sits on structure (keys, keywords) rather than every
// quoted byte screaming for attention.
const QColor kKey = QColor("#0b5fff");        // job names, mapping keys
const QColor kKeyword = QColor("#7a2da8");    // groovy keywords / yaml booleans
const QColor kString = QColor("#0a7a3b");
const QColor kNumber = QColor("#a8590c");
const QColor kComment = QColor("#7d8590");
const QColor kAnchor = QColor("#b03078");     // yaml anchors / aliases
const QColor kVar = QColor("#1f6feb");        // $VAR / ${VAR}

QTextCharFormat italicFormat(const QColor &c) {
    QTextCharFormat f;
    f.setForeground(c);
    f.setFontItalic(true);
    return f;
}

QTextCharFormat plain(const QColor &c) {
    QTextCharFormat f;
    f.setForeground(c);
    return f;
}

QTextCharFormat bold(const QColor &c) {
    QTextCharFormat f;
    f.setForeground(c);
    f.setFontWeight(QFont::DemiBold);
    return f;
}

}  // namespace

SyntaxHighlighter::SyntaxHighlighter(QTextDocument *parent) : QSyntaxHighlighter(parent) {
    m_commentFormat = italicFormat(kComment);
    buildYamlRules();
}

void SyntaxHighlighter::setLanguage(Language lang) {
    if (lang == m_language && !m_rules.empty()) {
        return;
    }
    m_language = lang;
    m_rules.clear();
    if (lang == Language::Yaml) {
        buildYamlRules();
    } else {
        buildGroovyRules();
    }
    rehighlight();
}

void SyntaxHighlighter::buildYamlRules() {
    // Order matters: comment last so it wins over anything matched
    // earlier on the same line.
    m_rules.push_back({QRegularExpression(QStringLiteral("^\\s*-?\\s*[\\w.\\-]+(?=\\s*:)")),
                       bold(kKey)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\"(?:[^\"\\\\]|\\\\.)*\"")), plain(kString)});
    m_rules.push_back({QRegularExpression(QStringLiteral("'(?:[^'\\\\]|\\\\.)*'")), plain(kString)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\\$\\{[^}]+\\}|\\$[A-Za-z_][\\w]*")),
                       plain(kVar)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\\b(?:true|false|null|yes|no|on|off|~)\\b")),
                       plain(kKeyword)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\\b\\d+(?:\\.\\d+)?\\b")), plain(kNumber)});
    m_rules.push_back({QRegularExpression(QStringLiteral("[&*][A-Za-z_][\\w-]*")), plain(kAnchor)});
    m_rules.push_back({QRegularExpression(QStringLiteral("#.*$")), m_commentFormat});
}

void SyntaxHighlighter::buildGroovyRules() {
    static const QString kKeywords = QStringLiteral(
        "\\b(?:pipeline|agent|any|none|label|docker|kubernetes|stages|stage|steps|"
        "when|environment|script|post|always|success|failure|unstable|aborted|"
        "changed|fixed|regression|cleanup|parameters|options|tools|input|matrix|"
        "axis|axes|excludes|exclude|sh|bat|powershell|pwsh|node|echo|dir|"
        "withCredentials|withEnv|catchError|build|timeout|retry|deleteDir|stash|"
        "unstash|checkout|scm|def|return|if|else|for|while|try|catch|finally|"
        "throw|new|this|true|false|null|in|as|instanceof|import)\\b");

    m_rules.push_back({QRegularExpression(kKeywords), bold(kKeyword)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\"(?:[^\"\\\\]|\\\\.)*\"")), plain(kString)});
    m_rules.push_back({QRegularExpression(QStringLiteral("'(?:[^'\\\\]|\\\\.)*'")), plain(kString)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\\$\\{[^}]+\\}|\\$[A-Za-z_][\\w]*")),
                       plain(kVar)});
    m_rules.push_back({QRegularExpression(QStringLiteral("\\b\\d+(?:\\.\\d+)?\\b")), plain(kNumber)});
    // `stage('X')` and `node('Y')` — emphasise the literal label.
    m_rules.push_back({QRegularExpression(QStringLiteral("(?<=stage\\()\\s*['\"][^'\"]+['\"]")),
                       bold(kKey)});
    m_rules.push_back({QRegularExpression(QStringLiteral("//[^\\n]*$")), m_commentFormat});
}

void SyntaxHighlighter::applyRules(const QString &text) {
    for (const auto &rule : m_rules) {
        auto it = rule.pattern.globalMatch(text);
        while (it.hasNext()) {
            const auto m = it.next();
            setFormat(m.capturedStart(), m.capturedLength(), rule.format);
        }
    }
}

void SyntaxHighlighter::applyMultilineComment(const QString &text) {
    if (m_language != Language::Groovy) {
        return;
    }
    setCurrentBlockState(0);
    int startIndex = 0;
    if (previousBlockState() != 1) {
        const auto m = m_blockCommentStart.match(text);
        startIndex = m.hasMatch() ? m.capturedStart() : -1;
    }
    while (startIndex >= 0) {
        const auto endMatch = m_blockCommentEnd.match(text, startIndex);
        int endIndex = endMatch.hasMatch() ? endMatch.capturedEnd() : -1;
        int length = 0;
        if (endIndex == -1) {
            setCurrentBlockState(1);
            length = text.length() - startIndex;
        } else {
            length = endIndex - startIndex;
        }
        setFormat(startIndex, length, m_commentFormat);
        const auto next = m_blockCommentStart.match(text, startIndex + length);
        startIndex = next.hasMatch() ? next.capturedStart() : -1;
    }
}

void SyntaxHighlighter::highlightBlock(const QString &text) {
    applyRules(text);
    applyMultilineComment(text);
}
