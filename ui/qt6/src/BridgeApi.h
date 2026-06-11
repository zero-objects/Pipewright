#pragma once

#include <QHash>
#include <QObject>
#include <QPointer>
#include <QQuickTextDocument>
#include <QString>
#include <QStringList>
#include <QTranslator>
#include <QVariantMap>
#include <qqmlintegration.h>

class QTextDocument;
class SyntaxHighlighter;

/// QML-facing facade over the `pipeline_ffi.h` C functions.
///
/// One Q_INVOKABLE per FFI entry point. Returns QString (UTF-8 JSON
/// or raw SVG); the C side's PipelineString allocation is freed
/// before each return. Errors come back as `{"error": "..."}` JSON
/// — QML decides how to surface them.
class BridgeApi : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    explicit BridgeApi(QObject *parent = nullptr);

    Q_INVOKABLE QString version() const;

    // The engine's supported platforms as a JSON array of keys. Single source
    // of truth for every platform picker (so the UI never drifts from the
    // engine's `PLATFORMS`).
    Q_INVOKABLE QStringList platforms() const;

    // Switch the UI-chrome language: load/install the Qt translator for
    // `locale` ("en" = none / source strings) and retranslate the live QML so
    // every `qsTr()` string updates in place. Paired with the app-wide language
    // selector that also drives the prose locale.
    Q_INVOKABLE void setLanguage(const QString &locale);

    /// Read the file at `path` and return `{ok: true, content: "…"}`
    /// or `{ok: false, error: "…"}` as a QML-native object. Replaces
    /// the QML `XMLHttpRequest`-on-`file://` path that Qt 6 disabled
    /// by default and that left users with an empty editor unless
    /// they set `QML_XHR_ALLOW_FILE_READ=1`.
    Q_INVOKABLE QVariantMap readFile(const QString &path) const;

    /// Best-effort search for the `pipeline` CLI binary. Tries (in
    /// order): `PATH`; `target/release/pipeline` / `target/debug/pipeline`
    /// walking up from `sourcePathHint`; `~/.cargo/bin/pipeline`;
    /// Homebrew's `/opt/homebrew/bin` and `/usr/local/bin`. Returns
    /// the first existing executable, or empty if nothing matches.
    Q_INVOKABLE QString discoverCliPath(const QString &sourcePathHint = QString()) const;

    /// Install (or update) the syntax highlighter on a QML
    /// `TextEdit`/`TextArea`'s document. `language` is one of
    /// `gitlab`, `github`, `jenkins`; `jenkins` enables the Groovy
    /// rules, everything else falls back to YAML.
    Q_INVOKABLE void installHighlighter(QQuickTextDocument *document,
                                        const QString &language);

    Q_INVOKABLE QString detectSourceKind(const QString &yaml) const;

    // Detection that prefers the file NAME over the content (the loaded path
    // is the strongest, least-forgeable signal). `path` may be empty.
    Q_INVOKABLE QString detectWithPath(const QString &path, const QString &yaml) const;

    // Whether `kind`'s pipelines can run locally in Docker:
    // `{runnable: bool, reason: "<why not>"}`. The Run tab disables the button
    // and shows the reason for translate-only platforms (argo, tekton, …).
    Q_INVOKABLE QString runSupport(const QString &kind) const;

    // The platform's conventional pipeline file name (".gitlab-ci.yml",
    // "Jenkinsfile", …) — what a save dialog should suggest. Sourced from
    // the engine so the 17-platform table never lives in QML.
    Q_INVOKABLE QString defaultFileName(const QString &kind) const;

    // The trailing `includeRoot` is the directory `include:` blocks
    // resolve relative to — typically the parent of the opened
    // pipeline file. Empty (= QML default) falls back to the no-op
    // fetcher, matching the pre-fix behaviour for callers that
    // don't have a path on disk.
    Q_INVOKABLE QString inspect(const QString &yaml,
                                const QString &sourceKind,
                                const QString &triggerEvent,
                                const QString &ref,
                                const QString &includeRoot = QString()) const;

    Q_INVOKABLE QString renderSvg(const QString &yaml,
                                  const QString &sourceKind,
                                  const QString &triggerEvent,
                                  const QString &ref,
                                  bool showScripts,
                                  bool showCapabilities,
                                  const QString &includeRoot = QString()) const;

    Q_INVOKABLE QString renderSvgWithLayout(const QString &yaml,
                                            const QString &sourceKind,
                                            const QString &triggerEvent,
                                            const QString &ref,
                                            bool showScripts,
                                            bool showCapabilities,
                                            const QString &includeRoot = QString()) const;

    // Human-readable runbook. Returns JSON shaped like
    // {overview, toc:[{title,anchor,summary}], jobs:[{anchor,title,html}], skipped}.
    // `locale` ("en"/"de"/…) selects the runbook prose language; empty = default.
    Q_INVOKABLE QString renderHuman(const QString &yaml,
                                    const QString &sourceKind,
                                    const QString &triggerEvent,
                                    const QString &ref,
                                    const QString &includeRoot = QString(),
                                    const QString &locale = QString()) const;

    Q_INVOKABLE QString capabilities(const QString &yaml,
                                     const QString &sourceKind,
                                     const QString &includeRoot = QString()) const;

    Q_INVOKABLE QString migrate(const QString &yaml,
                                const QString &sourceKind,
                                const QString &target,
                                const QString &includeRoot = QString()) const;

    Q_INVOKABLE QString composeRecipes(const QStringList &paths,
                                       const QString &target) const;

    // Browse the recipe registry: the embedded standard library plus any
    // user sources declared in the config at `configPath` (git sources cloned
    // into `cacheDir`). `query` filters on id/description/tags; `sort` is
    // "name" | "tag" | "source". Returns
    // `{"ok": true, "recipes": [{id, version, description, doc, tags,
    // requirements, inputs, outputs, jobs, source}], "warnings": […]}`.
    Q_INVOKABLE QString listRecipes(const QString &query,
                                    const QString &sort,
                                    const QString &configPath = QString(),
                                    const QString &cacheDir = QString()) const;

    // A generated, localized structural description (Markdown) of recipe
    // `recipeId`, via the prose doc mechanism. `locale` is e.g. "en" / "de".
    // Returns `{"ok": true, "markdown": "…"}` or `{"error": "…"}`.
    Q_INVOKABLE QString describeRecipe(const QString &recipeId,
                                       const QString &locale,
                                       const QString &configPath = QString(),
                                       const QString &cacheDir = QString()) const;

    // Apply recipe `recipeId` to the current pipeline `yaml` of platform
    // `sourceKind`: merge the recipe's jobs in and re-emit. The graph-edit
    // "apply recipe" operation. Returns `{"ok": true, "yaml": "…"}` or
    // `{"error": "…"}`.
    Q_INVOKABLE QString applyRecipe(const QString &yaml,
                                    const QString &sourceKind,
                                    const QString &recipeId,
                                    const QString &configPath = QString(),
                                    const QString &cacheDir = QString()) const;

    // Apply a value edit: replace the source span of the field identified by
    // `hub` (a diagram line's `data-hub` GhostId hex) with `newValue`. Returns
    // `{"ok": true, "yaml": "…"}` with the updated source — formatting and
    // comments preserved, only the edited span changes — or `{"error": "…"}`.
    Q_INVOKABLE QString editField(const QString &yaml,
                                  const QString &sourceKind,
                                  const QString &hub,
                                  const QString &newValue) const;

    // Duplicate / delete the construct at `hub` (a job or step) via the Hub-IR
    // and re-emit. Both return `{"ok": true, "yaml": "…"}` or `{"error": "…"}`.
    Q_INVOKABLE QString duplicate(const QString &yaml,
                                  const QString &sourceKind,
                                  const QString &hub) const;
    Q_INVOKABLE QString remove(const QString &yaml,
                               const QString &sourceKind,
                               const QString &hub) const;

    // Export the human-readable runbook in `format` ("md" | "html" | "doc").
    // Returns `{"ok": true, "content": "…", "ext": "md|html|rtf"}` JSON, or
    // `{"error": "…"}`.
    Q_INVOKABLE QString exportRunbook(const QString &yaml,
                                      const QString &sourceKind,
                                      const QString &format,
                                      const QString &locale = QString()) const;

    // Write `content` to `path`. Returns `{ok: true}` or `{ok: false, error}`.
    Q_INVOKABLE QVariantMap writeFile(const QString &path,
                                      const QString &content) const;

private:
    // One highlighter per document (Source tab, Migrate source pane,
    // Migrate target pane, …). QPointer<doc> keys auto-null when a
    // document is destroyed; we sweep nullified entries lazily on
    // install. The highlighter itself is parented to its document, so
    // Qt cleans it up when the document goes away.
    QHash<QTextDocument *, QPointer<SyntaxHighlighter>> m_highlighters;

    // The currently-installed UI-chrome translator (empty = English source).
    QTranslator m_translator;
};
