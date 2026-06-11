#include "BridgeApi.h"

#include "SyntaxHighlighter.h"

#include <pipeline_ffi.h>

#include <QByteArray>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QQmlEngine>
#include <QQuickTextDocument>
#include <QStandardPaths>
#include <QTextDocument>
#include <vector>

namespace {

// QML callers pass the opened pipeline file (or its dir); the FFI
// expects a *directory* that `include:` blocks resolve relative to.
// Normalise both forms so QML can simply hand over `sourcePath`.
QByteArray includeRootBytes(const QString &input) {
    if (input.isEmpty()) {
        return {};
    }
    const QFileInfo fi(input);
    if (fi.isFile()) {
        return fi.absolutePath().toUtf8();
    }
    return input.toUtf8();
}

}  // namespace

namespace {

QString takeFfiString(PipelineString *handle) {
    if (!handle) {
        return QStringLiteral("{\"error\":\"ffi returned null\"}");
    }
    const char *data = pipeline_string_data(handle);
    QString out = data ? QString::fromUtf8(data) : QString();
    pipeline_string_free(handle);
    return out;
}

}  // namespace

BridgeApi::BridgeApi(QObject *parent) : QObject(parent) {}

QString BridgeApi::version() const {
    return takeFfiString(pipeline_ffi_version());
}

void BridgeApi::setLanguage(const QString &locale) {
    // Swap the installed translator, then retranslate the live QML so every
    // qsTr() re-evaluates in place. "en" (or a missing .qm) → source strings.
    qApp->removeTranslator(&m_translator);
    if (locale != QStringLiteral("en")
        && m_translator.load(QStringLiteral(":/i18n/pipewright_%1").arg(locale))) {
        qApp->installTranslator(&m_translator);
    }
    if (QQmlEngine *engine = qmlEngine(this)) {
        engine->retranslate();
    }
}

QStringList BridgeApi::platforms() const {
    const QString json = takeFfiString(pipeline_platforms());
    QStringList out;
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (doc.isArray()) {
        for (const QJsonValue &v : doc.array()) {
            out << v.toString();
        }
    }
    return out;
}

QString BridgeApi::discoverCliPath(const QString &sourcePathHint) const {
    // 1) PATH lookup (handles `pipeline` installed system-wide).
    const QString fromPath = QStandardPaths::findExecutable(QStringLiteral("pipeline"));
    if (!fromPath.isEmpty()) {
        return fromPath;
    }

    auto isExec = [](const QString &p) {
        const QFileInfo fi(p);
        return fi.exists() && fi.isFile() && fi.isExecutable();
    };

    // 2) Walk up from a list of starting points, looking for a
    //    Cargo workspace with its built CLI under `target/`. We
    //    try both the opened source file (if any) AND the UI
    //    binary's own location — the latter covers the common dev
    //    flow where the user opens an *external* pipeline file but
    //    the CLI sits in the same workspace they built the UI from.
    QStringList walkRoots;
    if (!sourcePathHint.isEmpty()) {
        walkRoots << QFileInfo(sourcePathHint).absolutePath();
    }
    walkRoots << QCoreApplication::applicationDirPath();
    for (const QString &start : walkRoots) {
        QDir dir(start);
        for (int hops = 0; hops < 16; ++hops) {
            for (const QString sub : {QStringLiteral("target/release/pipeline"),
                                      QStringLiteral("target/debug/pipeline")}) {
                const QString candidate = dir.absoluteFilePath(sub);
                if (isExec(candidate)) {
                    return candidate;
                }
            }
            if (!dir.cdUp()) {
                break;
            }
        }
    }

    // 3) Common install locations.
    const QString home = QDir::homePath();
    for (const QString &candidate : {
             home + QStringLiteral("/.cargo/bin/pipeline"),
             QStringLiteral("/opt/homebrew/bin/pipeline"),
             QStringLiteral("/usr/local/bin/pipeline"),
         }) {
        if (isExec(candidate)) {
            return candidate;
        }
    }

    return {};
}

QVariantMap BridgeApi::readFile(const QString &path) const {
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) {
        return QVariantMap{
            {QStringLiteral("ok"), false},
            {QStringLiteral("error"), QStringLiteral("cannot open: %1").arg(f.errorString())},
        };
    }
    return QVariantMap{
        {QStringLiteral("ok"), true},
        {QStringLiteral("content"), QString::fromUtf8(f.readAll())},
    };
}

QVariantMap BridgeApi::writeFile(const QString &path, const QString &content) const {
    QFile f(path);
    if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
        return QVariantMap{
            {QStringLiteral("ok"), false},
            {QStringLiteral("error"), QStringLiteral("cannot write: %1").arg(f.errorString())},
        };
    }
    f.write(content.toUtf8());
    return QVariantMap{{QStringLiteral("ok"), true}};
}

QString BridgeApi::duplicate(const QString &yaml,
                             const QString &sourceKind,
                             const QString &hub) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray h = hub.toUtf8();
    return takeFfiString(pipeline_duplicate(y.constData(), k.constData(), h.constData()));
}

QString BridgeApi::remove(const QString &yaml,
                          const QString &sourceKind,
                          const QString &hub) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray h = hub.toUtf8();
    return takeFfiString(pipeline_delete(y.constData(), k.constData(), h.constData()));
}

QString BridgeApi::exportRunbook(const QString &yaml,
                                 const QString &sourceKind,
                                 const QString &format,
                                 const QString &locale) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray f = format.toUtf8();
    const QByteArray l = locale.toUtf8();
    return takeFfiString(pipeline_export_runbook_in(
        y.constData(), k.constData(), f.constData(), l.constData()));
}

void BridgeApi::installHighlighter(QQuickTextDocument *document, const QString &language) {
    if (!document) {
        return;
    }
    QTextDocument *doc = document->textDocument();
    if (!doc) {
        return;
    }
    // Sweep entries whose document was destroyed (QPointer auto-nulls).
    for (auto it = m_highlighters.begin(); it != m_highlighters.end();) {
        if (it.value().isNull()) {
            it = m_highlighters.erase(it);
        } else {
            ++it;
        }
    }
    auto it = m_highlighters.find(doc);
    if (it == m_highlighters.end() || it.value().isNull()) {
        // SyntaxHighlighter is parented to the document, so Qt cleans
        // it up when the document goes away — no manual delete needed.
        auto *hl = new SyntaxHighlighter(doc);
        it = m_highlighters.insert(doc, hl);
    }
    const auto lang = (language.compare(QStringLiteral("jenkins"), Qt::CaseInsensitive) == 0)
                          ? SyntaxHighlighter::Language::Groovy
                          : SyntaxHighlighter::Language::Yaml;
    it.value()->setLanguage(lang);
}

QString BridgeApi::detectSourceKind(const QString &yaml) const {
    const QByteArray y = yaml.toUtf8();
    return takeFfiString(pipeline_detect_source_kind(y.constData()));
}

QString BridgeApi::detectWithPath(const QString &path, const QString &yaml) const {
    const QByteArray p = path.toUtf8();
    const QByteArray y = yaml.toUtf8();
    return takeFfiString(pipeline_detect_with_path(p.constData(), y.constData()));
}

QString BridgeApi::runSupport(const QString &kind) const {
    const QByteArray k = kind.toUtf8();
    return takeFfiString(pipeline_run_support(k.constData()));
}

QString BridgeApi::defaultFileName(const QString &kind) const {
    const QByteArray k = kind.toUtf8();
    const QString json = takeFfiString(pipeline_default_file_name(k.constData()));
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    return doc.object().value(QStringLiteral("name")).toString(QStringLiteral("pipeline.yml"));
}

QString BridgeApi::inspect(const QString &yaml,
                           const QString &sourceKind,
                           const QString &triggerEvent,
                           const QString &ref,
                           const QString &includeRoot) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray t = triggerEvent.toUtf8();
    const QByteArray r = ref.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(pipeline_inspect_json(
        y.constData(), k.constData(), t.constData(), r.constData(), ir.constData()));
}

QString BridgeApi::renderSvg(const QString &yaml,
                             const QString &sourceKind,
                             const QString &triggerEvent,
                             const QString &ref,
                             bool showScripts,
                             bool showCapabilities,
                             const QString &includeRoot) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray t = triggerEvent.toUtf8();
    const QByteArray r = ref.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(pipeline_render_svg(
        y.constData(),
        k.constData(),
        t.constData(),
        r.constData(),
        showScripts ? 1 : 0,
        showCapabilities ? 1 : 0,
        ir.constData()));
}

QString BridgeApi::renderSvgWithLayout(const QString &yaml,
                                       const QString &sourceKind,
                                       const QString &triggerEvent,
                                       const QString &ref,
                                       bool showScripts,
                                       bool showCapabilities,
                                       const QString &includeRoot) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray t = triggerEvent.toUtf8();
    const QByteArray r = ref.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(pipeline_render_svg_with_layout(
        y.constData(),
        k.constData(),
        t.constData(),
        r.constData(),
        showScripts ? 1 : 0,
        showCapabilities ? 1 : 0,
        ir.constData()));
}

QString BridgeApi::renderHuman(const QString &yaml,
                               const QString &sourceKind,
                               const QString &triggerEvent,
                               const QString &ref,
                               const QString &includeRoot,
                               const QString &locale) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray l = locale.toUtf8();
    const QByteArray t = triggerEvent.toUtf8();
    const QByteArray r = ref.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(pipeline_render_html_in(
        y.constData(),
        k.constData(),
        l.constData(),
        t.constData(),
        r.constData(),
        ir.constData()));
}

QString BridgeApi::capabilities(const QString &yaml,
                                const QString &sourceKind,
                                const QString &includeRoot) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(
        pipeline_capabilities_json(y.constData(), k.constData(), ir.constData()));
}

QString BridgeApi::migrate(const QString &yaml,
                           const QString &sourceKind,
                           const QString &target,
                           const QString &includeRoot) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray t = target.toUtf8();
    const QByteArray ir = includeRootBytes(includeRoot);
    return takeFfiString(pipeline_migrate(
        y.constData(), k.constData(), t.constData(), ir.constData()));
}

QString BridgeApi::composeRecipes(const QStringList &paths,
                                  const QString &target) const {
    std::vector<QByteArray> backing;
    backing.reserve(static_cast<size_t>(paths.size()));
    std::vector<const char *> ptrs;
    ptrs.reserve(static_cast<size_t>(paths.size()));
    for (const auto &p : paths) {
        backing.push_back(p.toUtf8());
        ptrs.push_back(backing.back().constData());
    }
    const QByteArray t = target.toUtf8();
    return takeFfiString(pipeline_compose_recipes(
        ptrs.data(), ptrs.size(), t.constData()));
}

QString BridgeApi::listRecipes(const QString &query,
                               const QString &sort,
                               const QString &configPath,
                               const QString &cacheDir) const {
    const QByteArray q = query.toUtf8();
    const QByteArray s = sort.toUtf8();
    const QByteArray cp = configPath.toUtf8();
    const QByteArray cd = cacheDir.toUtf8();
    return takeFfiString(pipeline_list_recipes(
        q.constData(), s.constData(), cp.constData(), cd.constData()));
}

QString BridgeApi::describeRecipe(const QString &recipeId,
                                  const QString &locale,
                                  const QString &configPath,
                                  const QString &cacheDir) const {
    const QByteArray id = recipeId.toUtf8();
    const QByteArray l = locale.toUtf8();
    const QByteArray cp = configPath.toUtf8();
    const QByteArray cd = cacheDir.toUtf8();
    return takeFfiString(pipeline_describe_recipe(
        id.constData(), l.constData(), cp.constData(), cd.constData()));
}

QString BridgeApi::applyRecipe(const QString &yaml,
                               const QString &sourceKind,
                               const QString &recipeId,
                               const QString &configPath,
                               const QString &cacheDir) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray id = recipeId.toUtf8();
    const QByteArray cp = configPath.toUtf8();
    const QByteArray cd = cacheDir.toUtf8();
    return takeFfiString(pipeline_apply_recipe(
        y.constData(), k.constData(), id.constData(), cp.constData(), cd.constData()));
}

QString BridgeApi::editField(const QString &yaml,
                             const QString &sourceKind,
                             const QString &hub,
                             const QString &newValue) const {
    const QByteArray y = yaml.toUtf8();
    const QByteArray k = sourceKind.toUtf8();
    const QByteArray h = hub.toUtf8();
    const QByteArray v = newValue.toUtf8();
    return takeFfiString(pipeline_edit_field(
        y.constData(), k.constData(), h.constData(), v.constData()));
}
