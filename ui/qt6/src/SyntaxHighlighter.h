#pragma once

#include <QRegularExpression>
#include <QSyntaxHighlighter>
#include <QTextCharFormat>
#include <vector>

class QTextDocument;

/// Lightweight regex-based highlighter for the three source dialects
/// Pipewright reads: YAML (GitLab + GitHub workflows) and Groovy
/// (Jenkinsfiles). The rules are intentionally minimal — enough to
/// make keys, strings, comments and a handful of language keywords
/// pop, not a full grammar.
class SyntaxHighlighter : public QSyntaxHighlighter {
    Q_OBJECT

public:
    enum class Language { Yaml, Groovy };

    explicit SyntaxHighlighter(QTextDocument *parent = nullptr);

    void setLanguage(Language lang);

protected:
    void highlightBlock(const QString &text) override;

private:
    struct Rule {
        QRegularExpression pattern;
        QTextCharFormat format;
    };

    void buildYamlRules();
    void buildGroovyRules();
    void applyRules(const QString &text);
    void applyMultilineComment(const QString &text);

    Language m_language{Language::Yaml};
    std::vector<Rule> m_rules;

    // Multi-line `/* … */` handling for Groovy.
    QRegularExpression m_blockCommentStart{QStringLiteral("/\\*")};
    QRegularExpression m_blockCommentEnd{QStringLiteral("\\*/")};
    QTextCharFormat m_commentFormat;
};
