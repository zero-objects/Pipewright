//! SVG emission for the job DAG. Each node `<g>` carries `data-hub` (the
//! source `GhostId`, 64-char hex) and `data-name`, so the UI can hit-test a
//! click back to an IR node — and a future TGG-reverse editor can map an edit
//! on the SVG straight back to the graph element via [`GhostId::from_hex`].
//!
//! Alongside the SVG string we emit a [`DiagramLayout`] descriptor: the same
//! geometry as plain serialisable data, so the Qt/QML side can draw its own
//! hit-targets without re-parsing the SVG.

use std::fmt::Write as _;

use serde::Serialize;

use crate::layout::{layout, Layout};
use crate::model::Pipeline;

/// A positioned job box as plain data — the UI's click-target descriptor.
#[derive(Debug, Clone, Serialize)]
pub struct JobBox {
    pub name: String,
    /// Source hub `GhostId` in 64-char hex (parse back with `GhostId::from_hex`).
    pub hub: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A stage swimlane band for the UI to paint behind a column.
#[derive(Debug, Clone, Serialize)]
pub struct Lane {
    pub stage: String,
    pub x: f32,
    pub width: f32,
}

/// A dependency edge as a `(from, to)` pair of job hub hexes — for the QML side
/// to draw connectors natively (the SVG draws its own).
#[derive(Debug, Clone, Serialize)]
pub struct EdgeRef {
    pub from: String,
    pub to: String,
}

/// The diagram geometry as serialisable data (mirrors the SVG coordinates).
#[derive(Debug, Clone, Serialize)]
pub struct DiagramLayout {
    pub width: f32,
    pub height: f32,
    pub jobs: Vec<JobBox>,
    /// Stage swimlanes, left→right; empty for needs-only platforms.
    pub lanes: Vec<Lane>,
    /// Dependency edges as job-hub pairs.
    pub edges: Vec<EdgeRef>,
}

/// The diagram: the SVG markup plus its layout descriptor.
#[derive(Debug, Clone, Serialize)]
pub struct Diagram {
    pub svg: String,
    pub layout: DiagramLayout,
}

/// Render a pipeline to an SVG diagram and its layout descriptor.
#[must_use]
pub fn render_diagram(p: &Pipeline) -> Diagram {
    let l = layout(p);
    Diagram {
        svg: svg(p, &l),
        layout: descriptor(&l),
    }
}

/// The serialisable layout descriptor for a laid-out pipeline.
#[must_use]
fn descriptor(l: &Layout) -> DiagramLayout {
    DiagramLayout {
        width: l.width,
        height: l.height,
        jobs: l
            .nodes
            .iter()
            .map(|n| JobBox {
                name: n.name.clone(),
                hub: n.hub.hex(),
                x: n.x,
                y: n.y,
                w: n.w,
                h: n.h,
            })
            .collect(),
        lanes: l
            .lanes
            .iter()
            .map(|b| Lane {
                stage: b.stage.clone(),
                x: b.x,
                width: b.width,
            })
            .collect(),
        edges: l
            .edges
            .iter()
            .map(|e| EdgeRef {
                from: l.nodes[e.from].hub.hex(),
                to: l.nodes[e.to].hub.hex(),
            })
            .collect(),
    }
}

/// Emit the SVG markup for a laid-out pipeline as UML-style compartment nodes:
/// a header (job name + stage), a parameter compartment (image / env / …), and
/// a steps compartment (numbered commands). Every editable line carries its
/// source `data-hub` `GhostId` (and `data-field`), so a click maps to the IR.
#[must_use]
pub fn svg(p: &Pipeline, l: &Layout) -> String {
    let mut s = String::new();
    let (w, h) = (l.width.max(160.0), l.height.max(80.0));
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0} {h:.0}" width="{w:.0}" height="{h:.0}" font-family="-apple-system, Segoe UI, Roboto, sans-serif">"#
    );
    s.push_str(STYLE);
    s.push_str(DEFS);
    let _ = write!(
        s,
        r##"<rect width="{w:.0}" height="{h:.0}" fill="#fbfbfd"/>"##
    );

    // Stage swimlane bands (behind everything), with header labels.
    for (i, lane) in l.lanes.iter().enumerate() {
        let fill = if i % 2 == 0 { "#f3f5f9" } else { "#eef1f6" };
        let _ = write!(
            s,
            r#"<rect class="lane" x="{:.1}" y="0" width="{:.1}" height="{h:.0}" fill="{fill}"/>"#,
            lane.x, lane.width
        );
        if !lane.stage.is_empty() {
            let _ = write!(
                s,
                r#"<text class="lanehdr" x="{:.1}" y="18">{}</text>"#,
                lane.x + lane.width / 2.0,
                esc(&truncate(&lane.stage, 22))
            );
        }
    }

    // Edges first so nodes paint over their endpoints.
    for e in &l.edges {
        let (a, b) = (&l.nodes[e.from], &l.nodes[e.to]);
        let (x1, y1) = (a.x + a.w, a.y + a.h / 2.0);
        let (x2, y2) = (b.x, b.y + b.h / 2.0);
        let cx = (x2 - x1).abs().mul_add(0.5, x1);
        let _ = write!(
            s,
            r#"<path class="edge" d="M{x1:.1},{y1:.1} C{cx:.1},{y1:.1} {cx:.1},{y2:.1} {x2:.1},{y2:.1}" marker-end="url(#arrow)"/>"#
        );
    }

    if l.nodes.is_empty() {
        let _ = write!(
            s,
            r#"<text x="{:.0}" y="{:.0}" class="empty">no jobs in this pipeline's IR</text>"#,
            w / 2.0,
            h / 2.0,
        );
    }

    for (n, job) in l.nodes.iter().zip(&p.jobs) {
        node_svg(&mut s, n, job);
    }

    s.push_str("</svg>");
    s
}

/// Render one UML compartment node into `s`.
fn node_svg(s: &mut String, n: &crate::layout::NodeBox, job: &crate::model::Job) {
    use crate::layout::{HEADER_H, LINE_H, PAD};
    let lx = n.x + 12.0;
    let _ = write!(
        s,
        r#"<g class="node" data-hub="{}" data-name="{}"><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="9"/>"#,
        n.hub.hex(),
        esc(&n.name),
        n.x,
        n.y,
        n.w,
        n.h
    );
    let _ = write!(
        s,
        r#"<text x="{lx:.1}" y="{:.1}" class="title">{}</text>"#,
        n.y + 20.0,
        esc(&truncate(&n.name, 26))
    );
    if let Some(st) = &n.stage {
        let _ = write!(
            s,
            r#"<text x="{:.1}" y="{:.1}" class="stage" text-anchor="end">{}</text>"#,
            n.x + n.w - 12.0,
            n.y + 20.0,
            esc(&truncate(st, 14))
        );
    }
    let mut cy = n.y + HEADER_H;

    if !job.params.is_empty() {
        let _ = write!(
            s,
            r#"<line class="sep" x1="{:.1}" y1="{cy:.1}" x2="{:.1}" y2="{cy:.1}"/>"#,
            n.x,
            n.x + n.w
        );
        cy += PAD;
        for prm in &job.params {
            let _ = write!(
                s,
                r#"<text x="{lx:.1}" y="{:.1}" class="param" data-hub="{}" data-field="{}"><tspan class="pk">{}: </tspan>{}</text>"#,
                cy + 12.0,
                prm.id.hex(),
                esc(&prm.key),
                esc(&prm.key),
                esc(&truncate(&prm.value, 30))
            );
            cy += LINE_H;
        }
        cy += PAD;
    }

    if !job.steps.is_empty() {
        let _ = write!(
            s,
            r#"<line class="sep" x1="{:.1}" y1="{cy:.1}" x2="{:.1}" y2="{cy:.1}"/>"#,
            n.x,
            n.x + n.w
        );
        cy += PAD;
        for (idx, st) in job.steps.iter().enumerate() {
            let _ = write!(
                s,
                r#"<text x="{lx:.1}" y="{:.1}" class="step" data-hub="{}"><tspan class="num">{}</tspan> {}</text>"#,
                cy + 12.0,
                st.id.hex(),
                idx + 1,
                esc(&truncate(&st.label, 32))
            );
            cy += LINE_H;
        }
    }
    s.push_str("</g>");
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Minimal XML/SVG text escaping.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const STYLE: &str = r"<style>
.node rect{fill:#ffffff;stroke:#cdd2db;stroke-width:1.25}
.node:hover rect{stroke:#3b82f6;stroke-width:1.75}
.title{fill:#1b1f24;font-size:13px;font-weight:600}
.stage{fill:#0e7490;font-size:11px}
.sep{stroke:#e5e7eb;stroke-width:1}
.param{fill:#1b1f24;font-size:11px;font-family:ui-monospace,Menlo,monospace}
.param .pk{fill:#6b7280;font-family:-apple-system,Segoe UI,sans-serif}
.step{fill:#1b1f24;font-size:11px;font-family:ui-monospace,Menlo,monospace}
.step .num{fill:#9aa3af}
.edge{fill:none;stroke:#9aa3af;stroke-width:1.5}
.lanehdr{fill:#374151;font-size:12px;font-weight:600;text-anchor:middle}
.empty{fill:#9aa3af;font-size:13px;text-anchor:middle}
</style>";

const DEFS: &str = r##"<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#9aa3af"/></marker></defs>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Job, Pipeline};
    use seesaw_core::graph::GhostId;

    fn job(name: &str, needs: &[&str], steps: usize) -> Job {
        Job {
            id: GhostId::from_baseline(name),
            name: name.into(),
            stage: None,
            needs: needs.iter().map(|s| (*s).into()).collect(),
            condition: None,
            when: None,
            services: vec![],
            params: vec![],
            steps: (0..steps)
                .map(|i| crate::model::Step {
                    id: GhostId::from_baseline(&format!("{name}{i}")),
                    label: "x".into(),
                })
                .collect(),
            byte_start: None,
        }
    }

    #[test]
    fn svg_tags_each_node_with_its_ghostid() {
        let p = Pipeline {
            name: Some("CI".into()),
            stages: vec![],
            jobs: vec![job("build", &[], 2), job("test", &["build"], 1)],
            triggers: vec![],
        };
        let d = render_diagram(&p);
        assert!(d.svg.contains(&format!(
            "data-hub=\"{}\"",
            GhostId::from_baseline("build").hex()
        )));
        assert!(d.svg.contains("data-name=\"build\""));
        assert!(d.svg.contains(">test<") || d.svg.contains("test"));
        // One dependency edge → one path with an arrow marker.
        assert_eq!(d.svg.matches("marker-end=\"url(#arrow)\"").count(), 1);
        // Descriptor mirrors the nodes and carries the hex anchors.
        assert_eq!(d.layout.jobs.len(), 2);
        assert_eq!(d.layout.jobs[0].hub, GhostId::from_baseline("build").hex());
    }

    #[test]
    fn empty_pipeline_renders_honest_placeholder() {
        let p = Pipeline {
            name: Some("x".into()),
            stages: vec![],
            jobs: vec![],
            triggers: vec![],
        };
        let d = render_diagram(&p);
        assert!(d.svg.contains("no jobs"));
        assert!(d.layout.jobs.is_empty());
    }

    #[test]
    fn text_is_escaped() {
        let p = Pipeline {
            name: Some("x".into()),
            stages: vec![],
            jobs: vec![job("a<b>&c", &[], 0)],
            triggers: vec![],
        };
        let d = render_diagram(&p);
        assert!(d.svg.contains("a&lt;b&gt;&amp;c"));
        assert!(!d.svg.contains("data-name=\"a<b>"));
    }
}
