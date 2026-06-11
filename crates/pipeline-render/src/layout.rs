//! A layered (Sugiyama-style, left→right) layout for the job DAG. The render
//! `Pipeline` carries no coordinates — layout is a presentation concern, not
//! part of the IR — so this is where x/y are assigned. Ranks come from the
//! longest path through the `needs` edges, so a job always sits to the right of
//! everything it depends on; jobs sharing a rank stack vertically.
//!
//! The naive stage-column layout this replaces produced unreadable diagrams
//! (boxes overlapping, edges crossing the whole canvas); a proper rank
//! assignment keeps the flow legible.

use std::collections::HashMap;

use seesaw_core::graph::GhostId;

use crate::model::Pipeline;

/// Box geometry constants (CSS px). Boxes are uniform — step detail lives in
/// the runbook, not the diagram, which keeps the DAG itself readable.
use crate::model::Job;

/// Fixed node width (px). Long field text is truncated to fit.
pub(crate) const NODE_W: f32 = 248.0;
/// Title-compartment height (job name + stage).
pub(crate) const HEADER_H: f32 = 30.0;
/// Height of one body line (a param or a step).
pub(crate) const LINE_H: f32 = 17.0;
/// Vertical padding inside a compartment.
pub(crate) const PAD: f32 = 6.0;
const GAP_X: f32 = 84.0;
const GAP_Y: f32 = 26.0;
const MARGIN: f32 = 28.0;

/// Height of a UML node: header + (param compartment) + (steps compartment),
/// each compartment only present when it has content.
pub(crate) fn node_height(j: &Job) -> f32 {
    let mut h = HEADER_H;
    if !j.params.is_empty() {
        h += PAD.mul_add(2.0, j.params.len() as f32 * LINE_H);
    }
    if !j.steps.is_empty() {
        h += PAD.mul_add(2.0, j.steps.len() as f32 * LINE_H);
    }
    h
}

/// One positioned job box. The compartment content is read from the matching
/// `Pipeline::jobs[i]` at render time; this carries only geometry + identity.
#[derive(Debug, Clone)]
pub struct NodeBox {
    /// Source hub `GhostId` — the click-target / reverse-editor anchor.
    pub hub: GhostId,
    pub name: String,
    pub stage: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One DAG edge, as indices into [`Layout::nodes`].
#[derive(Debug, Clone, Copy)]
pub struct EdgeLine {
    pub from: usize,
    pub to: usize,
}

/// A stage swimlane: the band behind one stage column, with its header label.
#[derive(Debug, Clone)]
pub struct LaneBand {
    pub stage: String,
    pub x: f32,
    pub width: f32,
}

/// The positioned diagram: a canvas size, boxes, edges, and (when the platform
/// has stages) the swimlane bands the columns sit in.
#[derive(Debug, Clone)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<NodeBox>,
    pub edges: Vec<EdgeLine>,
    /// Stage swimlanes, left→right in declared order. Empty for needs-only
    /// platforms (no stages) — then columns rank by dependency depth instead.
    pub lanes: Vec<LaneBand>,
}

/// Vertical space reserved at the top for stage-lane header labels.
const LANE_HEADER_H: f32 = 26.0;

/// Assign ranks and coordinates to a pipeline's jobs.
#[must_use]
pub fn layout(p: &Pipeline) -> Layout {
    let n = p.jobs.len();
    if n == 0 {
        return Layout {
            width: MARGIN * 2.0,
            height: MARGIN * 2.0,
            nodes: vec![],
            edges: vec![],
            lanes: vec![],
        };
    }

    // Resolve `needs` (job names) to indices. Last writer wins on duplicate
    // names — rare, and the DAG stays well-defined either way.
    let name_to_idx: HashMap<&str, usize> = p
        .jobs
        .iter()
        .enumerate()
        .map(|(i, j)| (j.name.as_str(), i))
        .collect();

    // Dependency depth (longest needs-path) for every job — always computed:
    // it's the rank for needs-only platforms, and it orders undeclared stages.
    let mut nr = vec![None; n];
    let mut on_stack = vec![false; n];
    for i in 0..n {
        compute_rank(i, p, &name_to_idx, &mut nr, &mut on_stack);
    }
    let needs_rank: Vec<usize> = nr.into_iter().map(|x| x.unwrap_or(0)).collect();

    // Two ranking modes. Stage-based platforms get SWIMLANES — rank = the job's
    // stage column, in declared order (build → test → deploy, L→R). Needs-only
    // platforms fall back to dependency depth.
    let lane_names = lane_order(p, &needs_rank);
    let rank: Vec<usize> = if lane_names.is_empty() {
        needs_rank.clone()
    } else {
        p.jobs
            .iter()
            .map(|j| {
                j.stage
                    .as_ref()
                    .and_then(|s| lane_names.iter().position(|l| l == s))
                    .unwrap_or(lane_names.len())
            })
            .collect()
    };

    let max_rank = *rank.iter().max().unwrap_or(&0);
    let in_lanes = !lane_names.is_empty();
    let top = if in_lanes {
        MARGIN + LANE_HEADER_H
    } else {
        MARGIN
    };

    // Variable-height vertical stacking: walk jobs in model order and lay each
    // column out top-to-bottom with a running cursor, so taller nodes push their
    // column-mates down.
    let heights: Vec<f32> = p.jobs.iter().map(node_height).collect();
    let mut cursor = vec![top; max_rank + 1];
    let mut ys = vec![top; n];
    for i in 0..n {
        ys[i] = cursor[rank[i]];
        cursor[rank[i]] += heights[i] + GAP_Y;
    }

    let nodes: Vec<NodeBox> = p
        .jobs
        .iter()
        .enumerate()
        .map(|(i, j)| NodeBox {
            hub: j.id,
            name: j.name.clone(),
            stage: j.stage.clone(),
            x: (rank[i] as f32).mul_add(NODE_W + GAP_X, MARGIN),
            y: ys[i],
            w: NODE_W,
            h: heights[i],
        })
        .collect();

    let edges: Vec<EdgeLine> = p
        .jobs
        .iter()
        .enumerate()
        .flat_map(|(to, j)| {
            j.needs
                .iter()
                .filter_map(|dep| name_to_idx.get(dep.as_str()).copied())
                .map(move |from| EdgeLine { from, to })
        })
        .collect();

    let width = (max_rank as f32).mul_add(NODE_W + GAP_X, MARGIN * 2.0 + NODE_W);
    let col_bottom = cursor.iter().copied().fold(0.0_f32, f32::max);
    let height = col_bottom - GAP_Y + MARGIN;

    // Swimlane bands: one per column, spanning the column + half the gaps either
    // side, labelled with the stage (or "" for the trailing stageless column).
    let lanes: Vec<LaneBand> = if in_lanes {
        (0..=max_rank)
            .map(|r| LaneBand {
                stage: lane_names.get(r).cloned().unwrap_or_default(),
                x: (r as f32).mul_add(NODE_W + GAP_X, MARGIN) - GAP_X / 2.0,
                width: NODE_W + GAP_X,
            })
            .collect()
    } else {
        vec![]
    };

    Layout {
        width,
        height,
        nodes,
        edges,
        lanes,
    }
}

/// The stage swimlane order (left→right), or empty for a needs-only pipeline.
///
/// Declared `pipeline.stages` is authoritative when present; any stage that
/// appears only on jobs is appended. When NO stages are declared but jobs carry
/// `stage:` (gitlab-style), the order is derived from dependency depth — each
/// stage sits at the minimum `needs_rank` of its jobs — so the lanes still flow
/// in true execution order rather than job-declaration order.
fn lane_order(p: &Pipeline, needs_rank: &[usize]) -> Vec<String> {
    let mut lanes = p.stages.clone();
    let job_stages: Vec<&String> = p.jobs.iter().filter_map(|j| j.stage.as_ref()).collect();
    if lanes.is_empty() && job_stages.is_empty() {
        return vec![]; // needs-only platform: no swimlanes
    }

    if lanes.is_empty() {
        // Derive order from dependency depth: stage rank = min needs_rank of its
        // jobs; ties keep first-seen order.
        let mut by_depth: Vec<(usize, usize, String)> = Vec::new(); // (depth, seen, stage)
        for (seen, stage) in dedup_first_seen(&job_stages).into_iter().enumerate() {
            let depth = p
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, j)| j.stage.as_deref() == Some(stage.as_str()))
                .map(|(i, _)| needs_rank[i])
                .min()
                .unwrap_or(0);
            by_depth.push((depth, seen, stage));
        }
        by_depth.sort();
        return by_depth.into_iter().map(|(_, _, s)| s).collect();
    }

    // Declared order wins; append any extra job-only stages.
    for s in dedup_first_seen(&job_stages) {
        if !lanes.contains(&s) {
            lanes.push(s);
        }
    }
    lanes
}

/// Distinct strings in first-seen order.
fn dedup_first_seen(items: &[&String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in items {
        if seen.insert((*s).clone()) {
            out.push((*s).clone());
        }
    }
    out
}

/// Longest-path depth of job `i` over the `needs` DAG, memoised in `rank`.
fn compute_rank(
    i: usize,
    p: &Pipeline,
    name_to_idx: &HashMap<&str, usize>,
    rank: &mut [Option<usize>],
    on_stack: &mut [bool],
) -> usize {
    if let Some(r) = rank[i] {
        return r;
    }
    if on_stack[i] {
        // Cycle: break it by treating this back-edge as rank 0.
        return 0;
    }
    on_stack[i] = true;
    let r = p.jobs[i]
        .needs
        .iter()
        .filter_map(|dep| name_to_idx.get(dep.as_str()).copied())
        .map(|parent| compute_rank(parent, p, name_to_idx, rank, on_stack) + 1)
        .max()
        .unwrap_or(0);
    on_stack[i] = false;
    rank[i] = Some(r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Job, Pipeline};

    fn job(name: &str, needs: &[&str]) -> Job {
        Job {
            id: GhostId::from_baseline(name),
            name: name.into(),
            stage: None,
            needs: needs.iter().map(|s| (*s).into()).collect(),
            condition: None,
            when: None,
            services: vec![],
            params: vec![],
            steps: vec![],
            byte_start: None,
        }
    }

    fn pipe(jobs: Vec<Job>) -> Pipeline {
        Pipeline {
            name: Some("p".into()),
            stages: vec![],
            jobs,
            triggers: vec![],
        }
    }

    #[test]
    fn chain_ranks_increase_left_to_right() {
        // a → b → c : three columns.
        let l = layout(&pipe(vec![
            job("a", &[]),
            job("b", &["a"]),
            job("c", &["b"]),
        ]));
        assert!(l.nodes[0].x < l.nodes[1].x);
        assert!(l.nodes[1].x < l.nodes[2].x);
        assert_eq!(l.edges.len(), 2);
    }

    #[test]
    fn independent_jobs_share_a_column_and_stack() {
        let l = layout(&pipe(vec![job("a", &[]), job("b", &[])]));
        assert!((l.nodes[0].x - l.nodes[1].x).abs() < f32::EPSILON);
        assert!(l.nodes[0].y < l.nodes[1].y);
        assert!(l.edges.is_empty());
    }

    #[test]
    fn diamond_dependency_places_join_rightmost() {
        // a → b, a → c, b → d, c → d.
        let l = layout(&pipe(vec![
            job("a", &[]),
            job("b", &["a"]),
            job("c", &["a"]),
            job("d", &["b", "c"]),
        ]));
        let x = |i: usize| l.nodes[i].x;
        assert!(x(0) < x(1) && (x(1) - x(2)).abs() < f32::EPSILON && x(2) < x(3));
        assert_eq!(l.edges.len(), 4);
    }

    #[test]
    fn dependency_cycle_does_not_hang() {
        // a → b → a : guarded, must terminate.
        let l = layout(&pipe(vec![job("a", &["b"]), job("b", &["a"])]));
        assert_eq!(l.nodes.len(), 2);
    }

    #[test]
    fn empty_pipeline_yields_empty_canvas() {
        let l = layout(&pipe(vec![]));
        assert!(l.nodes.is_empty() && l.edges.is_empty());
    }
}
