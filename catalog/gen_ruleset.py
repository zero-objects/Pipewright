#!/usr/bin/env python3
"""Lower the rule manifests into seesaw RuleSetSpec JSON.

catalog/rules/<platform>.toml is a rule manifest — what rules exist.
This lowers each to a concrete seesaw `RuleSpec` and emits a loadable
catalog/rules/<platform>.ruleset.json.

A TGG rule set is bidirectional: the engine derives CST->IR and
IR->CST from the SAME rules (l_pattern / r_pattern are equal). There
is no separate "forward" or "backward" rule set.

CST convention (what the seeder produces — point 2): every
`cst:Mapping` that represents an IR construct is tagged
`construct = "<kind>"` — a job body, an `artifacts:` block, a
service item-mapping, the root, all alike. Tagging the *mapping*
(not the key-entry) covers both keyed and sequence-item constructs
uniformly. Key-entries keep `key`; scalars keep `text` +
`parent_key` (the key of the entry / list they belong to).

Because `construct` is a local tag, a rule anchors at
`cst:Mapping[construct=C]` and never walks from the root — the
per-platform structural classification is the seeder's job.

Graph model (a): a scalar field becomes a `hub:attr{name,value}`
child; a ref field becomes a sub-construct child node. Every rule
creates fresh child nodes or links shared anchors — never binds
onto a shared anchor.
"""
import json
import pathlib
import tomllib

CAT = pathlib.Path(__file__).parent
REFINES = "tgg:refines"
SPANS = [["span_start", "prov_byte_start"], ["span_end", "prov_byte_end"]]

# Platforms whose seeder canonicalises `X|list<X>` construct fields
# (single mapping → one-item sequence; see seed_top_level_with_list_fields
# + classify::*::LIST_CONSTRUCT_KEYS). For these, the ambiguous single
# mapping_node shape is DROPPED so seq_mapping_nodes is the SOLE bijective
# rule both directions. A platform must be wired in its seeder BEFORE being
# added here, else single-source construct-union fields lose forward coverage.
CANONICAL_LIST_PLATFORMS = {"gitlab"}


def lit(name, value):
    return {"name": name, "matcher": {"type": "literal", "value": value}}


# NOTE: a regex-".*" "attribute exists" gate on MC does NOT work for a
# first-class field rule: MC is created in the BACKWARD direction, and a
# created node may carry only LITERAL attrs (rc8 rejects a non-literal R
# constraint: NonLiteralRAttrUnsupported). The presence gate lives in emit
# instead — empty-valued first-class attrs (absent fields propagated as ""
# via collect_propagated_attrs' unwrap_or_default) are simply not emitted.


def n(nid, kind, *constraints):
    return {"id": nid, "kind": kind, "constraints": list(constraints)}


def e(kind, s, t):
    return {"kind": kind, "source_node_id": s, "target_node_id": t}


def corr(l, r, binds=(), role=None, kind=None):
    """Correspondence link. `role` (rc7):
      * "Establishes" — this rule CREATES the R-side node.
      * "References"  — the R-side node already exists (context);
                        the link may still carry a span anchor
                        (rc7 decouples context-classification from
                        the bindings, so a References corr can bind
                        spans for reverse-direction findability).
      * None          — rc6 fallback: empty bindings ⟹ context,
                        non-empty ⟹ creation.

    `kind` (default REFINES): the correspondence-node TYPE. rc8 roots a
    created node's GhostId in its corr node, whose id is
    `from_parent(anchor_hub, corrL, kind, {})` — so AT MOST ONE created cst
    node can hang off a given (hub, kind) pair. To anchor SEVERAL created
    nodes on the SAME hub node (e.g. a flattened wrapper: the `steps:` entry
    AND its sequence both belong to hub:pipeline), give each a DISTINCT kind
    encoding its structural role. The engine treats corr kinds generically
    (corrL/corrR edges + role/bindings drive behaviour, not the kind name).
    """
    link = {
        "l_node_id": l,
        "r_node_id": r,
        "kind": kind or REFINES,
        "attribute_bindings": [
            {"l_attr_name": a, "r_attr_name": b, "transformation": "identity"}
            for a, b in binds
        ],
    }
    if role is not None:
        link["role"] = role
    return link


def rule(name, rank, doc, l_nodes, l_edges, r_nodes, r_edges, links, nacs=None):
    return {
        "name": name,
        "rank": rank,
        "documentation": doc,
        "l_pattern": {"nodes": l_nodes, "edges": l_edges},
        "r_pattern": {"nodes": r_nodes, "edges": r_edges},
        "correspondence_links": links,
        "nacs": nacs or [],
    }


# value-scalar bindings for a hub:attr: name from the key, value
# from the scalar text, plus provenance.
ATTR_BINDS = [["parent_key", "name"], ["text", "value"], *SPANS]


def construct_rule(platform, ir_construct, cst_construct=None):
    """cst:Mapping[construct=<cst>] -> hub:<ir> node.

    Honours the `[<ir>.maps].<platform>` aliases: the IR groups
    related CST construct names under one hub identity (drone:
    `["step", "step_docker", "step_kubernetes", ...] → hub:job`).
    When `cst_construct` differs from `ir_construct` the rule's
    L-pattern matches the CST name the seeder actually emits,
    while the R-side still creates `hub:<ir>`. SPANS corr lets all
    such rules collapse onto one CST mapping in reverse — the
    GhostId hash sees the same construct constraint and the same
    span anchors, so we don't fork a step into duplicate CST
    nodes per alias."""
    cst = cst_construct or ir_construct
    if cst == ir_construct:
        rule_name = f"R_{platform}_{ir_construct}"
    else:
        rule_name = f"R_{platform}_{ir_construct}_from_{cst}"
    return rule(
        rule_name,
        90,
        f"{platform}: cst:Mapping[construct={cst}] <-> hub:{ir_construct}",
        [n("MC", "cst:Mapping", lit("construct", cst))],
        [],
        [n("hubC", f"hub:{ir_construct}")],
        [],
        # name→name propagates the construct's first-class name (the lifted
        # parent entry key, MC.name) onto hub:<ir>.name. Absent for unnamed
        # constructs → "" (harmless; emit skips empty). rc8 re-architecture.
        [corr("MC", "hubC", [["name", "name"], *SPANS])],
    )


def nested_steps_to_jobs_rule(platform, list_key, cst_construct):
    """Single BIJECTIVE rule mapping a flat-step list-shaped CST
    onto the canonical github-style nested IR.

    Forward:
        cst:Mapping[pipeline] -has_child-> ME[key=<list_key>] -value_of->
            Sequence -has_child-> SequenceItem -value_of-> cst:Mapping[<cst_construct>]
                          ↓
        hub:pipeline -has_job-> hub:job -has_step-> hub:step

    Reverse: mirror — the IR triangle uniquely reconstructs the
    drone-style `steps: [...]` list. Replaces the broken
    composition of `R_<plat>_step` + `R_<plat>_job_from_step` +
    `R_<plat>_pipeline_steps_steps` + `job_step_link_rule` —
    those compete with each other on reverse and produce
    malformed `jobs:` output (each platform fragment writing its
    own piece of structure). The single-rule form has no
    competition: one match → one creation in either direction.

    Ownership strategy: this rule is
    the SOLE creator of `hub:job` and the pipeline->job->step
    nesting. `hub:pipeline` and `hub:step` are References (context)
    — created by R_<plat>_pipeline and R_<plat>_step. The competing
    `job` construct/field/implicit rules (which alias the SAME step
    mapping onto hub:job) are suppressed in gen_ruleset via
    `bijective_owned_cst_tags`, so there is exactly one job-creation
    path:
        hub:pipeline (References) --has_job--> hub:job (Establishes)
        hub:job (Establishes)     --has_step--> hub:step (References)

    REVERSIBILITY — the crucial modelling point: job and step are
    two distinct hub nodes, so they MUST anchor on two distinct CST
    nodes, or the L<->R swap collapses both roles onto one created
    node. hub:job therefore anchors on the SequenceItem (IT, the
    list slot), hub:step on the inner Mapping (IM, the slot's
    content). Forward: IT->hubJ creates the job, IM->hubS references
    the step that R_<plat>_step already made. Reverse: hubJ->IT
    creates the list item, hubS->IM references the step mapping that
    R_<plat>_step_rev creates — same hub:step anchor, same GhostId,
    so no competing IM creation and no retraction oscillation.
    (Earlier both anchored on IM: reverse then had Establishes(job)
    AND References(step) on the one IM node, oscillating with
    R_<plat>_step_rev — both creating IM at different anchors.)"""
    return rule(
        _ident(f"R_{platform}_{list_key}_to_nested_jobs"),
        48,  # below construct (90) + field (50): pipeline & step exist first
        f"{platform}: pipeline.{list_key} list <-> hub:pipeline.has_job.has_step",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", list_key)),
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("IM", "cst:Mapping", lit("construct", cst_construct)),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "IM"),
        ],
        [
            n("hubP", "hub:pipeline"),
            n("hubJ", "hub:job"),
            n("hubS", "hub:step"),
        ],
        [
            e("hub:has_job", "hubP", "hubJ"),
            e("hub:has_step", "hubJ", "hubS"),
        ],
        [
            # rc7: References corrs carry the span anchor (SPANS) so
            # they stay findable in BOTH directions, while role —
            # not empty bindings — marks them context. This is the
            # exact rc7 unblock (report §1c / §2): the earlier
            # empty-binding context-corr lost the reverse anchor and
            # looped; now the anchor is preserved.
            corr("MC", "hubP", SPANS, role="References"),  # hub:pipeline: R_<plat>_pipeline made it
            # The `steps:` MappingEntry S and its Sequence SEQ are the
            # pipeline-level WRAPPER for the has_job collection (ONE per
            # pipeline, shared across the N per-item matches). They have no
            # hub pendant — the hub nests jobs directly via has_job, no
            # wrapper node — so the wrapper is a SHARED SINGLETON over N
            # jobs: a non-bijective N:1 flattening, exactly like the
            # concept-trigger path. An Establishes corr would mint a
            # duplicate hub:pipeline per wrapper (distinct corr kind ⇒
            # distinct GhostId). Declare them REFERENCES onto the pipeline
            # instead: rc8 routes a reference-corr target into the MATCH,
            # not nodes_to_create (compile.rs §4b), satisfying the invariant
            # without minting hub nodes. Forward the wrapper is matched
            # context; the per-item job (IT↔hubJ) stays the bijective
            # Establishes creation. Backward reconstruction of the `steps:`
            # wrapper is emit's job (it knows drone nests jobs as a list).
            corr("S", "hubP", SPANS, role="References"),
            corr("SEQ", "hubP", SPANS, role="References"),
            # hub:job anchors on the SequenceItem (IT) — a DISTINCT CST
            # node from the step mapping (IM). This is the reversibility
            # fix: job and step are two hub nodes, so they need two
            # distinct CST anchors. With both on IM, the reverse swap
            # collapsed Establishes(job) + References(step) onto the one
            # created IM node — a role contradiction that oscillated with
            # R_<plat>_step_rev (both creating IM). Anchoring hubJ on IT
            # means reverse creates IT (the list item) from hub:job and
            # merely *references* the IM that R_<plat>_step_rev creates.
            corr("IT", "hubJ", SPANS, role="Establishes"),  # hub:job: this rule creates it (from the list item)
            corr("IM", "hubS", SPANS, role="References"),  # hub:step: R_<plat>_step + field rules made it
        ],
    )


def bitbucket_default_steps_rule():
    """bitbucket F2 stage 1: `pipelines.default` `- step:` list <-> jobs.

    CST shape — one hop deeper than azure's `jobs:` list (each item wraps
    its body in a single-key `step:` mapping):

        cst:Mapping[pipeline]            (the `pipelines:` value)
          -has_child-> S[key=default] -value_of-> SEQ
          -has_child-> IT -value_of-> WM            (the `- step:` wrapper)
          -has_child-> WE[key=step] -value_of-> IM[construct=job]

    Hub shape mirrors R_azure_pipeline_jobs_jobs (the AttrCollection job
    containment: attr{name=jobs}+collection+has_item), so
    `normalize_job_containment` recognises bitbucket's JobForm for
    migration; `prov_key=default` keeps the surface key for reverse.

    Corr roles follow the azure rule: the construct ends (pipeline, job)
    are References — their identity rules (R_bitbucket_pipeline /
    R_bitbucket_job) establish them; this rule establishes the field
    attr, the collection, and the wrapper chain. IT/WM/WE all anchor on
    rk — the same multi-anchor idiom as seq_block_attr's IT/IM double
    anchor on hub:item; reverse recreates each chain node (key literals
    become creation_attrs, so emit gets `default:` and `- step:` back).

    The seeder only tags `construct=job` on INLINE step bodies under
    `pipelines.default` (incl. `- parallel:` groups; alias-valued steps
    seed as scalars and stay meta), so this rule cannot over-match the
    branches/tags/custom sub-pipelines (F2 stage-2 residual)."""
    return rule(
        "R_bitbucket_pipeline_jobs_default",
        50,
        "bitbucket: pipelines.default `- step:` list <-> hub:pipeline jobs collection",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", "default")),
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            # `wrapper=step` (seeder-set) differentiates WM's GhostId from
            # IM's: both would otherwise hash as (anchor=rk, value_of,
            # cst:Mapping, spans) in reverse and collapse into ONE node —
            # the wrapper then vanishes from the emitted YAML.
            n("WM", "cst:Mapping", lit("wrapper", "step")),
            n("WE", "cst:MappingEntry", lit("key", "step")),
            n("IM", "cst:Mapping", lit("construct", "job")),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "WM"),
            e("cst:has_child", "WM", "WE"),
            e("cst:value_of", "WE", "IM"),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", "jobs"), lit("prov_key", "default"), lit("vkind", "seq")),
            n("coll", "hub:collection"),
            n("rk", "hub:job"),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "rk"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("SEQ", "coll", SPANS),
            # IT/WM/WE anchor on the SAME rk as IM — four CST nodes for one
            # hub:job. DISTINCT corr kinds (the seqblock_wrap idiom) are
            # load-bearing twice over: (a) identical (refines, rk, SPANS)
            # signatures unify by GhostId — azure WANTS that (its IT≡IM
            # skeleton-share puts job mappings directly under the list),
            # but here it inverted the chain (job under SEQ, wrapper under
            # job); (b) tgg:refines context-walks (the IM candidate search)
            # must not traverse the wrapper corrs — with plain refines the
            # fresh wrapper picked up `construct=job` via attrs_to_set and
            # the rule re-fired on it (4 items from 2 jobs).
            #
            # DISTINCT hub-attr names per binding (not prov_byte_*): all
            # three bind onto the ONE rk, whose canonical prov_byte_* the
            # construct rule sets from the step BODY — sharing the name made
            # forward ping-pong SetAttr(28)/SetAttr(42) forever (the
            # uncached cascade never saturated). prov_-prefixed → invisible
            # to hub_signature; reverse still gets per-item identity attrs.
            corr("IT", "rk", [["span_start", "prov_item_start"], ["span_end", "prov_item_end"]], kind="bb_step_item"),
            corr("WM", "rk", [["span_start", "prov_wrap_start"], ["span_end", "prov_wrap_end"]], kind="bb_step_wrapper"),
            corr("WE", "rk", [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]], kind="bb_step_entry"),
            corr("IM", "rk", SPANS, role="References"),
        ],
    )


def bitbucket_parallel_group_rule():
    """bitbucket F2 stage 2a, identity half: the `- parallel:` wrapper
    mapping (seeder-tagged `wrapper=parallel`) IS the group —
    `hub:item{vkind=parallel}` — via a plain 1:1 refines corr, exactly
    like a construct rule (R_bitbucket_job for IM). The containment rule
    then only REFERENCES grp: a creation-R node carrying SEVERAL
    establishing corrs is minted ONCE PER CORR (four grp instances in
    the first attempt) — the reference-corr match-routing (rc8) is what
    de-duplicates, and it needs exactly this separate creator."""
    return rule(
        "R_bitbucket_parallel_group",
        90,
        "bitbucket: cst:Mapping[wrapper=parallel] <-> hub:item{vkind=parallel}",
        [n("PWM", "cst:Mapping", lit("wrapper", "parallel"))],
        [],
        [n("grp", "hub:item", lit("vkind", "parallel"))],
        [],
        [corr("PWM", "grp", SPANS)],
    )


def bitbucket_selector_group_rule(selector, group_vkind):
    """bitbucket F2 stage 2b, identity half: the named entry under a
    selector map (`branches: main:`, seeder-tagged `selector=<sel>`) IS
    the group — `hub:item{vkind=<singular>, name=<key>}`. Same
    separate-creator reasoning as [`bitbucket_parallel_group_rule`];
    the seeder tag on the ENTRY keeps this a 1:1 single-anchor rule (a
    BM-context variant would need a References corr on grp, which rc8
    routes into the match — the rule would then never fire forward)."""
    return rule(
        _ident(f"R_bitbucket_{selector}_group"),
        90,
        f"bitbucket: {selector}-map entry <-> hub:item{{vkind={group_vkind}, name=<key>}}",
        [n("BE", "cst:MappingEntry", lit("selector", selector))],
        [],
        [n("grp", "hub:item", lit("vkind", group_vkind))],
        [],
        [corr("BE", "grp", [["key", "name"], *SPANS])],
    )


def bitbucket_parallel_steps_rule():
    """bitbucket F2 stage 2a: `- parallel:` groups under `pipelines.default`.

    Same chain as the direct rule with one more wrapper level; the hub
    side inserts a GROUP node — `hub:item{vkind=parallel}` — between the
    collection and the jobs. That structural discriminator is what makes
    the reverse direction unambiguous against the direct rule (both
    would otherwise share `coll -has_item-> rk` and reverse couldn't
    decide which chain to rebuild — the fan-in problem). S↔attr and
    SEQ↔coll deliberately repeat the direct rule's corrs: identical
    (kind, anchor, bindings) signatures GhostId-unify, so both rules
    share ONE attr+collection skeleton (the azure skeleton-share).

    Per-anchor multi-corr discipline as in the direct rule: distinct
    corr kinds per chain node, distinct prov_* attr names per writer
    (grp: 4 writers, rk: 3 writers — see stage-1 lessons in
    reverse-corr discipline)."""
    isp = [["span_start", "prov_item_start"], ["span_end", "prov_item_end"]]
    wsp = [["span_start", "prov_wrap_start"], ["span_end", "prov_wrap_end"]]
    esp = [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]]
    qsp = [["span_start", "prov_seq_start"], ["span_end", "prov_seq_end"]]
    return rule(
        "R_bitbucket_pipeline_jobs_default_parallel",
        50,
        "bitbucket: pipelines.default `- parallel:` steps <-> hub jobs under item{vkind=parallel}",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", "default")),
            n("SEQ", "cst:Sequence"),
            n("GIT", "cst:SequenceItem"),
            n("PWM", "cst:Mapping", lit("wrapper", "parallel")),
            n("PWE", "cst:MappingEntry", lit("key", "parallel")),
            n("PSEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("WM", "cst:Mapping", lit("wrapper", "step")),
            n("WE", "cst:MappingEntry", lit("key", "step")),
            n("IM", "cst:Mapping", lit("construct", "job")),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "GIT"),
            e("cst:value_of", "GIT", "PWM"),
            e("cst:has_child", "PWM", "PWE"),
            e("cst:value_of", "PWE", "PSEQ"),
            e("cst:has_child", "PSEQ", "IT"),
            e("cst:value_of", "IT", "WM"),
            e("cst:has_child", "WM", "WE"),
            e("cst:value_of", "WE", "IM"),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", "jobs"), lit("prov_key", "default"), lit("vkind", "seq")),
            n("coll", "hub:collection"),
            n("grp", "hub:item", lit("vkind", "parallel")),
            n("rk", "hub:job"),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "grp"),
            e("hub:has_item", "grp", "rk"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("SEQ", "coll", SPANS),
            corr("GIT", "grp", isp, kind="bb_par_item"),
            # grp is ESTABLISHED by R_bitbucket_parallel_group (PWM↔grp
            # refines) — this rule only references it, exactly like IM↔rk.
            # A creation-R node with several establishing corrs is minted
            # once PER CORR (we measured four grp instances) — the
            # References routing is the de-duplication.
            corr("PWM", "grp", SPANS, role="References"),
            corr("PWE", "grp", esp, kind="bb_par_entry"),
            corr("PSEQ", "grp", qsp, kind="bb_par_seq"),
            corr("IT", "rk", isp, kind="bb_step_item"),
            corr("WM", "rk", wsp, kind="bb_step_wrapper"),
            corr("WE", "rk", esp, kind="bb_step_entry"),
            corr("IM", "rk", SPANS, role="References"),
        ],
    )


# bitbucket event selectors: `pipelines.<selector>` is a MAP of named
# sub-pipelines (branch glob / tag glob / custom name), each a `- step:` list.
# (selector key, hub attr name, group vkind)
BITBUCKET_SELECTORS = [
    ("branches", "branch_jobs", "branch"),
    ("tags", "tag_jobs", "tag"),
    ("bookmarks", "bookmark_jobs", "bookmark"),
    ("pull-requests", "pr_jobs", "pull-request"),
    ("custom", "custom_jobs", "custom"),
]


def bitbucket_selector_steps_rule(selector, attr_name, group_vkind):
    """bitbucket F2 stage 2b: `pipelines.<selector>.<name>` step lists.

    Each named entry (branch glob, tag glob, custom name) becomes a
    GROUP — `hub:item{vkind=<selector-singular>}` with the entry key as
    its `name` (the BE↔grp key↔name binding, exactly the
    implicit_containment idiom) — and its `- step:` jobs hang under the
    group. Distinct hub attr names (branch_jobs/tag_jobs/…) keep the
    five selectors AND the default list disjoint backward (prov_key
    re-derives the surface key). Discipline as in the parallel rule."""
    isp = [["span_start", "prov_item_start"], ["span_end", "prov_item_end"]]
    wsp = [["span_start", "prov_wrap_start"], ["span_end", "prov_wrap_end"]]
    esp = [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]]
    qsp = [["span_start", "prov_seq_start"], ["span_end", "prov_seq_end"]]
    return rule(
        _ident(f"R_bitbucket_pipeline_jobs_{selector}"),
        50,
        f"bitbucket: pipelines.{selector}.<name> `- step:` list <-> hub jobs under item{{vkind={group_vkind}}}",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", selector)),
            n("BM", "cst:Mapping", lit("wrapper", selector)),
            n("BE", "cst:MappingEntry", lit("selector", selector)),
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("WM", "cst:Mapping", lit("wrapper", "step")),
            n("WE", "cst:MappingEntry", lit("key", "step")),
            n("IM", "cst:Mapping", lit("construct", "job")),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "BM"),
            e("cst:has_child", "BM", "BE"),
            e("cst:value_of", "BE", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "WM"),
            e("cst:has_child", "WM", "WE"),
            e("cst:value_of", "WE", "IM"),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", attr_name), lit("prov_key", selector), lit("vkind", "map")),
            n("coll", "hub:collection"),
            n("grp", "hub:item", lit("vkind", group_vkind)),
            n("rk", "hub:job"),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "grp"),
            e("hub:has_item", "grp", "rk"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("BM", "coll", SPANS),
            # grp is ESTABLISHED by R_bitbucket_<selector>_group (BE↔grp,
            # key↔name) — this rule only references it (same de-dup
            # reasoning as the parallel rule; bindings mirror the creator's).
            corr("BE", "grp", [["key", "name"], *SPANS], role="References"),
            corr("SEQ", "grp", qsp, kind="bb_sel_seq"),
            corr("IT", "rk", isp, kind="bb_step_item"),
            corr("WM", "rk", wsp, kind="bb_step_wrapper"),
            corr("WE", "rk", esp, kind="bb_step_entry"),
            corr("IM", "rk", SPANS, role="References"),
        ],
    )


def bitbucket_selector_parallel_steps_rule(selector, attr_name, group_vkind):
    """bitbucket F2 stage 2c: `- parallel:` groups INSIDE a selector list
    (`pipelines.branches.main: [- parallel: [- step: …]]`).

    Hub shape nests the parallel group under the selector group:
    attr{<sel>_jobs} → coll → grp_sel{vkind=<sel>} → grp_par{vkind=
    parallel} → rk. Both group nodes are References (their identity
    rules R_bitbucket_<sel>_group / R_bitbucket_parallel_group establish
    them — the per-corr-minting lesson); this rule establishes the
    chain skeleton only. Discipline as in the sibling rules."""
    isp = [["span_start", "prov_item_start"], ["span_end", "prov_item_end"]]
    wsp = [["span_start", "prov_wrap_start"], ["span_end", "prov_wrap_end"]]
    esp = [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]]
    qsp = [["span_start", "prov_seq_start"], ["span_end", "prov_seq_end"]]
    return rule(
        _ident(f"R_bitbucket_pipeline_jobs_{selector}_parallel"),
        50,
        f"bitbucket: pipelines.{selector}.<name> `- parallel:` steps <-> hub jobs under item{{vkind={group_vkind}}}/item{{vkind=parallel}}",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", selector)),
            n("BM", "cst:Mapping", lit("wrapper", selector)),
            n("BE", "cst:MappingEntry", lit("selector", selector)),
            n("SEQ", "cst:Sequence"),
            n("GIT", "cst:SequenceItem"),
            n("PWM", "cst:Mapping", lit("wrapper", "parallel")),
            n("PWE", "cst:MappingEntry", lit("key", "parallel")),
            n("PSEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("WM", "cst:Mapping", lit("wrapper", "step")),
            n("WE", "cst:MappingEntry", lit("key", "step")),
            n("IM", "cst:Mapping", lit("construct", "job")),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "BM"),
            e("cst:has_child", "BM", "BE"),
            e("cst:value_of", "BE", "SEQ"),
            e("cst:has_child", "SEQ", "GIT"),
            e("cst:value_of", "GIT", "PWM"),
            e("cst:has_child", "PWM", "PWE"),
            e("cst:value_of", "PWE", "PSEQ"),
            e("cst:has_child", "PSEQ", "IT"),
            e("cst:value_of", "IT", "WM"),
            e("cst:has_child", "WM", "WE"),
            e("cst:value_of", "WE", "IM"),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", attr_name), lit("prov_key", selector), lit("vkind", "map")),
            n("coll", "hub:collection"),
            n("gsel", "hub:item", lit("vkind", group_vkind)),
            n("gpar", "hub:item", lit("vkind", "parallel")),
            n("rk", "hub:job"),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "gsel"),
            e("hub:has_item", "gsel", "gpar"),
            e("hub:has_item", "gpar", "rk"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("BM", "coll", SPANS),
            corr("BE", "gsel", [["key", "name"], *SPANS], role="References"),
            corr("SEQ", "gsel", qsp, kind="bb_sel_seq"),
            corr("GIT", "gpar", isp, kind="bb_par_item"),
            corr("PWM", "gpar", SPANS, role="References"),
            corr("PWE", "gpar", esp, kind="bb_par_entry"),
            corr("PSEQ", "gpar", qsp, kind="bb_par_seq"),
            corr("IT", "rk", isp, kind="bb_step_item"),
            corr("WM", "rk", wsp, kind="bb_step_wrapper"),
            corr("WE", "rk", esp, kind="bb_step_entry"),
            corr("IM", "rk", SPANS, role="References"),
        ],
    )


def bitbucket_parallel_expanded_group_rule():
    """Identity half for the EXPANDED parallel form
    (`- parallel: {fail-fast: …, steps: […]}`): the item wrapper mapping
    (seeder-tagged `wrapper=parallel_expanded`) IS the group —
    `hub:item{vkind=parallel_expanded}`. The distinct vkind keeps the
    list-form and expanded-form containment rules disjoint BACKWARD
    (same hub shape would re-derive both chains — fan-in)."""
    return rule(
        "R_bitbucket_parallel_expanded_group",
        90,
        "bitbucket: cst:Mapping[wrapper=parallel_expanded] <-> hub:item{vkind=parallel_expanded}",
        [n("PWM", "cst:Mapping", lit("wrapper", "parallel_expanded"))],
        [],
        [n("grp", "hub:item", lit("vkind", "parallel_expanded"))],
        [],
        [corr("PWM", "grp", SPANS)],
    )


def bitbucket_parallel_expanded_fail_fast_rule(value):
    """`fail-fast: <bool>` on the expanded parallel group, captured as an
    INLINE `fail_fast=<value>` LITERAL on the group item (content-bearing
    — in the hub signature). One rule per boolean value: the literal on
    grp doubles as the backward existence gate — a binding variant fired
    on EVERY expanded group and emitted an empty `fail-fast:` on groups
    without the field (no exists-matcher; a regex gate would deadlock
    forward, where this very rule sets the attr).

    PWE/PM deliberately REPEAT the containment rule's corr kinds AND
    binding names — identical signatures GhostId-unify, so both rules
    share ONE entry+mapping skeleton (a private bb_ffx_* pair minted
    duplicate chains; the emit walker then split steps and fail-fast
    across the two and dropped one — first-reach-wins)."""
    esp = [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]]
    msp = [["span_start", "prov_map_start"], ["span_end", "prov_map_end"]]
    return rule(
        _ident(f"R_bitbucket_parallel_expanded_fail_fast_{value}"),
        50,
        f"bitbucket: parallel_expanded `fail-fast: {value}` <-> hub:item.fail_fast={value}",
        [
            n("PWM", "cst:Mapping", lit("wrapper", "parallel_expanded")),
            n("PWE", "cst:MappingEntry", lit("key", "parallel")),
            n("PM", "cst:Mapping"),
            n("FFE", "cst:MappingEntry", lit("key", "fail-fast")),
            n("FFS", "cst:Scalar", lit("text", value)),
        ],
        [
            e("cst:has_child", "PWM", "PWE"),
            e("cst:value_of", "PWE", "PM"),
            e("cst:has_child", "PM", "FFE"),
            e("cst:value_of", "FFE", "FFS"),
        ],
        [n("grp", "hub:item", lit("vkind", "parallel_expanded"), lit("fail_fast", value))],
        [],
        [
            corr("PWM", "grp", SPANS, role="References"),
            corr("PWE", "grp", esp, kind="bb_par_entry"),
            corr("PM", "grp", msp, kind="bb_parx_map"),
            corr("FFE", "grp", [["span_start", "prov_ff_entry_start"], ["span_end", "prov_ff_entry_end"]], kind="bb_ffx_entry"),
            corr("FFS", "grp", [["span_start", "prov_ff_start"], ["span_end", "prov_ff_end"]], kind="bb_ffx_scalar"),
        ],
    )


def _bitbucket_parallel_expanded_containment(name, doc, prefix_nodes, prefix_edges, prefix_corrs, hub_prefix_nodes, hub_prefix_edges, host_var):
    """Shared tail for the expanded-parallel containment rules: the chain
    from the `- parallel:` item (GIT) through the expanded mapping
    (PWE→PM→PSE[steps]→PSEQ) into the `- step:` wrappers, contained as
    `… → grp{vkind=parallel_expanded} → rk`. `host_var` is the hub node
    the group hangs under (coll for default, gsel for selectors)."""
    isp = [["span_start", "prov_item_start"], ["span_end", "prov_item_end"]]
    wsp = [["span_start", "prov_wrap_start"], ["span_end", "prov_wrap_end"]]
    esp = [["span_start", "prov_entry_start"], ["span_end", "prov_entry_end"]]
    qsp = [["span_start", "prov_seq_start"], ["span_end", "prov_seq_end"]]
    msp = [["span_start", "prov_map_start"], ["span_end", "prov_map_end"]]
    ssp = [["span_start", "prov_sentry_start"], ["span_end", "prov_sentry_end"]]
    nodes = prefix_nodes + [
        n("GIT", "cst:SequenceItem"),
        n("PWM", "cst:Mapping", lit("wrapper", "parallel_expanded")),
        n("PWE", "cst:MappingEntry", lit("key", "parallel")),
        n("PM", "cst:Mapping"),
        n("PSE", "cst:MappingEntry", lit("key", "steps")),
        n("PSEQ", "cst:Sequence"),
        n("IT", "cst:SequenceItem"),
        n("WM", "cst:Mapping", lit("wrapper", "step")),
        n("WE", "cst:MappingEntry", lit("key", "step")),
        n("IM", "cst:Mapping", lit("construct", "job")),
    ]
    edges = prefix_edges + [
        e("cst:value_of", "GIT", "PWM"),
        e("cst:has_child", "PWM", "PWE"),
        e("cst:value_of", "PWE", "PM"),
        e("cst:has_child", "PM", "PSE"),
        e("cst:value_of", "PSE", "PSEQ"),
        e("cst:has_child", "PSEQ", "IT"),
        e("cst:value_of", "IT", "WM"),
        e("cst:has_child", "WM", "WE"),
        e("cst:value_of", "WE", "IM"),
    ]
    r_nodes = hub_prefix_nodes + [
        n("grp", "hub:item", lit("vkind", "parallel_expanded")),
        n("rk", "hub:job"),
    ]
    r_edges = hub_prefix_edges + [
        e("hub:has_item", host_var, "grp"),
        e("hub:has_item", "grp", "rk"),
    ]
    corrs = prefix_corrs + [
        corr("GIT", "grp", isp, kind="bb_par_item"),
        corr("PWM", "grp", SPANS, role="References"),
        corr("PWE", "grp", esp, kind="bb_par_entry"),
        corr("PM", "grp", msp, kind="bb_parx_map"),
        corr("PSE", "grp", ssp, kind="bb_parx_steps"),
        corr("PSEQ", "grp", qsp, kind="bb_par_seq"),
        corr("IT", "rk", isp, kind="bb_step_item"),
        corr("WM", "rk", wsp, kind="bb_step_wrapper"),
        corr("WE", "rk", esp, kind="bb_step_entry"),
        corr("IM", "rk", SPANS, role="References"),
    ]
    return rule(_ident(name), 50, doc, nodes, edges, r_nodes, r_edges, corrs)


def bitbucket_parallel_expanded_steps_rule():
    """Expanded parallel under `pipelines.default` (stage 2e)."""
    return _bitbucket_parallel_expanded_containment(
        "R_bitbucket_pipeline_jobs_default_parallel_expanded",
        "bitbucket: pipelines.default expanded `- parallel:{steps}` <-> hub jobs under item{vkind=parallel_expanded}",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", "default")),
            n("SEQ", "cst:Sequence"),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "GIT"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("SEQ", "coll", SPANS),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", "jobs"), lit("prov_key", "default"), lit("vkind", "seq")),
            n("coll", "hub:collection"),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
        ],
        "coll",
    )


def bitbucket_selector_parallel_expanded_steps_rule(selector, attr_name, group_vkind):
    """Expanded parallel inside a selector list (stage 2e)."""
    qsp = [["span_start", "prov_seq_start"], ["span_end", "prov_seq_end"]]
    return _bitbucket_parallel_expanded_containment(
        f"R_bitbucket_pipeline_jobs_{selector}_parallel_expanded",
        f"bitbucket: pipelines.{selector}.<name> expanded `- parallel:{{steps}}` <-> hub jobs under item{{vkind={group_vkind}}}/item{{vkind=parallel_expanded}}",
        [
            n("MC", "cst:Mapping", lit("construct", "pipeline")),
            n("S", "cst:MappingEntry", lit("key", selector)),
            n("BM", "cst:Mapping", lit("wrapper", selector)),
            n("BE", "cst:MappingEntry", lit("selector", selector)),
            n("SEQ", "cst:Sequence"),
        ],
        [
            e("cst:has_child", "MC", "S"),
            e("cst:value_of", "S", "BM"),
            e("cst:has_child", "BM", "BE"),
            e("cst:value_of", "BE", "SEQ"),
            e("cst:has_child", "SEQ", "GIT"),
        ],
        [
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("BM", "coll", SPANS),
            corr("BE", "gsel", [["key", "name"], *SPANS], role="References"),
            corr("SEQ", "gsel", qsp, kind="bb_sel_seq"),
        ],
        [
            n("hubC", "hub:pipeline"),
            n("attr", "hub:attr", lit("name", attr_name), lit("prov_key", selector), lit("vkind", "map")),
            n("coll", "hub:collection"),
            n("gsel", "hub:item", lit("vkind", group_vkind)),
        ],
        [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "gsel"),
        ],
        "gsel",
    )


def job_name_from_nested_step_rule(platform, list_key, cst_construct, name_key):
    """Name the synthesised hub:job from the flat step's `name:` key.

    The bijective wrapper creates an anonymous hub:job per list item
    (anchored on the SequenceItem IT). Named-job target platforms
    (github, gitlab, circleci) key their `jobs:` map by the job name,
    so an anonymous hub:job reverse-emits as a malformed
    `jobs: { steps: ... }` block (the field name leaks in as the
    key). This rule lifts the step item's `name:` scalar onto
    hub:job.name so the cross-platform job key is well-defined.

    It anchors on IT (References the wrapper's hub:job) and on IM
    (References hub:step, so the inner mapping is context — not an
    orphan that the reverse pass would re-create and fork). Only the
    name attribute `a` is Established. Reverse re-materialises the
    `name:` entry under the step mapping. NOTE: the matching
    `step.name` field rule is suppressed for wrapper platforms (see
    gen_ruleset), so the name round-trips through hub:job alone and
    no duplicate `name:` entry is emitted."""
    return rule(
        _ident(f"R_{platform}_jobname_from_{cst_construct}_{name_key}"),
        46,  # after wrapper (48) creates hub:job, before generic fields fade
        f"{platform}: {cst_construct} item `{name_key}` <-> hub:job.name (nested)",
        [
            n("IT", "cst:SequenceItem"),
            n("IM", "cst:Mapping", lit("construct", cst_construct)),
            n("S", "cst:MappingEntry", lit("key", name_key)),
            n("SC", "cst:Scalar"),
        ],
        [
            e("cst:value_of", "IT", "IM"),
            e("cst:has_child", "IM", "S"),
            e("cst:value_of", "S", "SC"),
        ],
        [
            n("hubJ", "hub:job"),
            n("hubS", "hub:step"),
            n("a", "hub:attr", lit("name", name_key)),
        ],
        # The has_step edge is LOAD-BEARING: it pins hubJ to the
        # specific hubS that lives in the same list item. Without it
        # the matcher pairs every hub:job with every hub:step
        # (n*m matches), and in reverse every step's `name:` collapses
        # to one job's value. The wrapper created this edge at higher
        # rank, so it exists when this rule fires.
        [e("hub:has_step", "hubJ", "hubS"), e("hub:has_attr", "hubJ", "a")],
        [
            corr("IT", "hubJ", SPANS, role="References"),  # wrapper made hub:job
            corr("IM", "hubS", SPANS, role="References"),  # R_<plat>_step made hub:step (keeps IM context in reverse)
            corr("SC", "a", ATTR_BINDS, role="Establishes"),  # this rule creates the name attr
            # The `name:` MappingEntry S is the syntactic wrapper around the
            # name scalar — the VALUE rides hub:job.name = the attr `a`, so S
            # has no hub pendant of its own. Declare it a REFERENCE onto the
            # job (compile.rs §4b routes it into the MATCH, not
            # nodes_to_create) — forward it is matched context under the step
            # item; backward the `name:` entry is re-materialised by emit
            # alongside the step mapping. (Same forward-effective treatment
            # as the steps wrapper and the concept-trigger path.)
            corr("S", "hubJ", SPANS, role="References"),
        ],
    )


def job_step_link_rule(platform, cst_construct):
    """`hub:job -has_step-> hub:step` edge for platforms whose
    seeder co-creates BOTH hub:job AND hub:step from the same CST
    mapping (drone, woodpecker, buildkite, bitbucket: each step
    item IS a self-contained job that wraps a single step).

    Sandra's heuristic: map every platform onto the MOST COMPLEX
    canonical IR (github's `pipeline → has_job → job → has_step →
    step`). Flat-step platforms then materialise the nesting:
    each step item produces both hub:job (the wrapper) and
    hub:step (the run-content), connected by has_step. github's
    own paths produce the same shape natively, so cross-platform
    round-trips converge on the nested form.

    L-pattern: cst:Mapping[construct=<cst>] (single anchor — the
    same node both construct rules already corr to).

    R-pattern: hub:job -has_step-> hub:step (the two ends already
    exist as separate construct-rule creations on the same MC;
    SPANS corr makes their GhostIds merge with those creations,
    so this rule just adds the edge)."""
    return rule(
        _ident(f"R_{platform}_job_step_link_from_{cst_construct}"),
        85,
        f"{platform}: cst:Mapping[construct={cst_construct}] -> hub:job -has_step-> hub:step",
        [n("MC", "cst:Mapping", lit("construct", cst_construct))],
        [],
        [n("hubJ", "hub:job"), n("hubS", "hub:step")],
        [e("hub:has_step", "hubJ", "hubS")],
        [
            corr("MC", "hubJ", SPANS),
            corr("MC", "hubS", SPANS),
        ],
    )


def user_comment_rule(platform, construct):
    """`# foo` user comments — one rule per construct so a comment
    that lives under any tagged mapping (pipeline, job, step, …)
    survives. Forward lifts text+spans onto a fresh `hub:comment`;
    reverse re-materialises the cst:UserComment under the same
    construct it came from.
    """
    return rule(
        _ident(f"R_{platform}_{construct}_user_comment"),
        85,
        f"{platform}: user `# foo` comment under {construct} <-> hub:comment lexical node",
        [
            n("MC", "cst:Mapping", lit("construct", construct)),
            n("UC", "cst:UserComment"),
        ],
        [e("cst:has_child", "MC", "UC")],
        [n("hubC", f"hub:{construct}"), n("HC", "hub:comment")],
        [e("hub:has_comment", "hubC", "HC")],
        [
            corr("MC", "hubC", SPANS),  # SPANS so MC unifies with construct_rule's MC in reverse
            corr(
                "UC",
                "HC",
                [["text", "text"], *SPANS],
            ),
        ],
    )


def parent_key_name_rule(platform, ir_construct, cst_construct=None):
    """Map key → `hub:<ir>.name`. Replaces the seeder-synthesised
    `cst:CarrierComment[target_field=name]` whenever a construct
    lives as a map-entry value (travis `jobs.lint:`, github
    `jobs.build:`, circleci `workflows.<wf>:`, …).

    `cst_construct` honours `[<ir>.maps].<platform>` aliases so the
    rule anchors on the CST tag the seeder actually emits.

    L-pattern: cst:MappingEntry[key=*] -value_of-> cst:Mapping[construct=<cst>]
    R-pattern: hub:<ir> -has_attr-> hub:attr[name=name, value=<the key>]

    Corr binds `ME.key -> a.value`. No-op for constructs whose
    parent CST node is a `cst:SequenceItem` (drone flat steps,
    buildkite steps, …) — the L-pattern doesn't match a list
    item. Safe to emit unconditionally per (platform, ir, cst-alias)."""
    cst = cst_construct or ir_construct
    if cst == ir_construct:
        rname = f"R_{platform}_{ir_construct}_name_from_parent_key"
    else:
        rname = f"R_{platform}_{ir_construct}_from_{cst}_name_from_parent_key"
    return rule(
        _ident(rname),
        47,
        f"{platform}: parent map-entry key <-> hub:{ir_construct}.name (cst={cst})",
        [
            n("ME", "cst:MappingEntry"),
            n("MC", "cst:Mapping", lit("construct", cst)),
        ],
        [e("cst:value_of", "ME", "MC")],
        [n("hubC", f"hub:{ir_construct}"), n("a", "hub:attr", lit("name", "name"))],
        [e("hub:has_attr", "hubC", "a")],
        [
            corr("MC", "hubC", SPANS),
            corr("ME", "a", [["key", "value"], *SPANS]),
        ],
    )


def list_implicit_containment_rule(
    platform, parent, child_field, child_kind,
    parent_cst_construct=None, child_cst_construct=None,
):
    """Variant of `implicit_containment_rule` for child constructs
    that live as items in a sequence (drone `steps: [{…}, {…}]`)
    rather than entries in a map (github `jobs.<name>: {…}`).

    L-pattern:
        cst:Mapping[parent] -has_child-> ME -value_of-> Sequence
          -has_child-> SequenceItem -value_of-> Mapping[child]

    No key↔name binding (the item has no key), so list-shaped
    children rely on their own internal `name:` field rule for
    identity (already covered by field_rule emission)."""
    cst_parent = parent_cst_construct or parent
    cst_child = child_cst_construct or child_kind
    l_nodes = [
        n("MC", "cst:Mapping", lit("construct", cst_parent)),
        n("S", "cst:MappingEntry"),
        n("SEQ", "cst:Sequence"),
        n("IT", "cst:SequenceItem"),
        n("IM", "cst:Mapping", lit("construct", cst_child)),
    ]
    l_edges = [
        e("cst:has_child", "MC", "S"),
        e("cst:value_of", "S", "SEQ"),
        e("cst:has_child", "SEQ", "IT"),
        e("cst:value_of", "IT", "IM"),
    ]
    r_nodes = [n("hubC", f"hub:{parent}"), n("rk", f"hub:{child_kind}")]
    r_edges = [e(f"hub:has_{child_kind}", "hubC", "rk")]
    corrs = [corr("MC", "hubC", SPANS), corr("IM", "rk", SPANS)]
    base_id = f"R_{platform}_{parent}_{child_field}_list_implicit"
    if cst_parent != parent or cst_child != child_kind:
        base_id = (
            f"R_{platform}_{parent}_from_{cst_parent}_{child_field}"
            f"_list_implicit_from_{cst_child}"
        )
    return rule(
        _ident(base_id),
        40,
        f"{platform}: list-implicit containment "
        f"{parent}[cst={cst_parent}].{child_field} <-> {child_kind}[cst={cst_child}]",
        l_nodes,
        l_edges,
        r_nodes,
        r_edges,
        corrs,
    )


def implicit_containment_rule(
    platform, parent, child_field, child_kind,
    parent_cst_construct=None, child_cst_construct=None,
):
    """Tag-only structural containment: a parent-tagged mapping directly
    holds a child-tagged sub-mapping via `has_child → value_of` — no
    intermediate key constraint at the manifest level (GitLab-style
    implicit containers: top-level jobs, services-as-items, …).

    The mapping-entry key is bound to the child sub-construct's
    `name` hub:attr satellite. Forward: the seeder tags the
    mapping with `construct=<child>` AND synthesises a name carrier
    (`@hub:<child>.name=<entry_key>`) which the attr_carrier_rule
    lifts to a hub:attr{name=name, value=<entry_key>}. The implicit
    rule then binds the entry's `key` ↔ that satellite's `value`.

    Reverse: hub:<parent> + hub:has_<child> + hub:<child> with
    hub:attr{name=name, value=X} satellite → emit a cst:MappingEntry
    with key=X around the cst:Mapping[construct=<child>]. Closes the
    job-name-key gap so a `build:` key in the source survives
    roundtrip as a real key, not just a `# @hub:job.name=build`
    carrier comment."""
    # Top-level constructs (jobs and pipelines) are entered by NAME:
    # `build:` in a gitlab pipeline IS the job's name. Sub-constructs
    # (artifact, step, cache, …) are entered by FIELD name: `artifacts:`
    # under a job is the IR field name, not the artifact's name (the
    # name lives inside, as `name: <value>`). Bind key ↔ name only
    # for the name-keyed kinds; otherwise the rule would synthesise
    # a spurious `hub:attr{name=name, value=<field key>}` that
    # clobbers the real name carrier.
    name_keyed = child_kind in ("job", "pipeline")
    # `cst_*` overrides honour the platform's classify-derived
    # tags (`[<ir>.maps].<platform>` ∩ classify table). When the
    # CST tag the seeder emits differs from the IR construct name
    # (drone's "step" tag → hub:job), the implicit rule must
    # anchor on the actual tag or it never matches.
    cst_parent_local = parent_cst_construct or parent
    cst_child_local = child_cst_construct or child_kind
    l_nodes = [
        n("MC", "cst:Mapping", lit("construct", cst_parent_local)),
        n("S", "cst:MappingEntry"),
        n("IM", "cst:Mapping", lit("construct", cst_child_local)),
    ]
    l_edges = [e("cst:has_child", "MC", "S"), e("cst:value_of", "S", "IM")]
    # SPANS on the MC/IM corrs so they become creation-R nodes in reverse
    # (literals → creation_attrs). Without bindings they were context-R:
    # rc6 would then context-match ANY existing cst:Mapping for IM and run
    # `attrs_to_set { construct: <C> }` on it, accidentally tagging
    # unrelated mappings (the env-block cst:Mapping in drone fixtures
    # picked up `construct=step` this way, turning into a phantom step).
    if name_keyed:
        # Name-entered constructs (jobs/pipelines): typed-edge child + a
        # `name` satellite carrying the entry key. The entry S is anchored
        # via the key↔name binding (S↔a).
        r_nodes = [
            n("hubC", f"hub:{parent}"),
            n("rk", f"hub:{child_kind}"),
            n("a", "hub:attr", lit("name", "name")),
        ]
        r_edges = [
            e(f"hub:has_{child_kind}", "hubC", "rk"),
            e("hub:has_attr", "rk", "a"),
        ]
        # Forward: S.key (= "build") flows into a.value. Reverse: a.value
        # (= "build" from the satellite) becomes S.key.
        corrs = [
            # Both constructs (parent + child) are established by their own
            # identity rules; this containment rule only References them and
            # creates the entry/name satellite (S↔a). rc8 corr-rooted idiom.
            corr("MC", "hubC", SPANS, role="References"),
            corr("IM", "rk", SPANS, role="References"),
            corr("S", "a", [["key", "value"]]),
        ]
    else:
        # Field-entered (fixed-key) sub-constructs (artifacts:/cache:/…):
        # the entry key IS the IR field name, NOT the construct's name, so
        # there is no name satellite to anchor S. Use the attr+coll form
        # (== the mapping_node single shape): the field-attr anchors S
        # (S↔attr); the collection has no cst:Sequence to anchor it, so it
        # roots on S too (S↔coll); the construct is matched via IM↔rk. Every
        # created node carries a corr (rc8 created-node invariant).
        r_nodes = [
            n("hubC", f"hub:{parent}"),
            n("attr", "hub:attr", lit("name", child_field), lit("prov_key", child_field)),
            n("coll", "hub:collection"),
            n("rk", f"hub:{child_kind}"),
        ]
        r_edges = [
            e("hub:has_attr", "hubC", "attr"),
            e("hub:has_value", "attr", "coll"),
            e("hub:has_item", "coll", "rk"),
        ]
        corrs = [
            # Parent + child constructs are References (their identity rules
            # establish them); this rule establishes only the field-attr,
            # its collection, and the entry anchor. rc8 corr-rooted idiom.
            corr("MC", "hubC", SPANS, role="References"),
            corr("S", "attr", SPANS),
            corr("S", "coll", SPANS),
            corr("IM", "rk", SPANS, role="References"),
        ]
    base_id = f"R_{platform}_{parent}_{child_field}_implicit"
    if cst_parent_local != parent or cst_child_local != child_kind:
        base_id = (
            f"R_{platform}_{parent}_from_{cst_parent_local}_{child_field}"
            f"_implicit_from_{cst_child_local}"
        )
    return rule(
        _ident(base_id),
        40,
        f"{platform}: implicit containment {parent}[cst={cst_parent_local}]."
        f"{child_field} <-> {child_kind}[cst={cst_child_local}]",
        l_nodes,
        l_edges,
        r_nodes,
        r_edges,
        corrs,
    )


def _ident(s):
    return "".join(c if (c.isalnum() or c == "_") else "_" for c in s)


def _read_classify_tags(platform):
    """Extract the set of `construct=<X>` tags that the seeder's
    classify table for `platform` may produce. Single source of
    truth lives in `crates/pipeline-tgg-seeder/src/classify/<plat>.rs`
    — a small Rust file with `pub const CONSTRUCT_KEYS: &[(&str, &str)]
    = &[ ("<key>", "<ir_construct>"), … ];`. We parse it textually
    rather than committing a generated JSON alongside, because the
    seeder file is hand-curated per platform and the catalog
    generator needs to stay in sync without a separate dump step.

    Returns the set of `<ir_construct>` values. The IR root tag
    `pipeline` is always included — every platform's
    `open_pipeline` tags the document mapping with
    `construct=pipeline` regardless of classify."""
    import re
    path = pathlib.Path(__file__).resolve().parent.parent / "crates" / "pipeline-tgg-seeder" / "src" / "classify" / f"{platform}.rs"
    tags = {"pipeline"}
    try:
        text = path.read_text()
    except FileNotFoundError:
        return tags
    for m in re.finditer(r'\("[^"]+"\s*,\s*"([^"]+)"\)', text):
        tags.add(m.group(1))
    return tags


def field_rule(platform, construct, field, key, shape, ref_kind, cst_construct=None,
               single_forward_only=False):
    """Lower one manifest rule to a RuleSpec (or a list, for unions).

    `single_forward_only` (mapping_node shape only): make the single
    construct mapping FORWARD-EFFECTIVE — IM↔rk becomes a References corr
    so the BACKWARD direction does not reconstruct it. Set when the field
    is an `X | list<X>` union whose seq_mapping_nodes sibling already owns
    backward reconstruction (the single form would otherwise compete and
    collapse a multi-item collection to one mapping). The construct's own
    construct_rule still creates hub:<ref_kind> forward.

    `cst_construct` lets the field rule match a CST tag different
    from the IR construct name (`[<ir>.maps].<platform>` aliases —
    drone's "step" CST → hub:job means the field rules for
    `job.image` need L = construct=step, not construct=job, or
    they never anchor on what the seeder actually produces). When
    unset, behaves exactly as before (CST tag = IR construct name)."""
    cst = cst_construct or construct
    if cst == construct:
        name = _ident(f"R_{platform}_{construct}_{field}_{key}")
    else:
        name = _ident(f"R_{platform}_{construct}_from_{cst}_{field}_{key}")
    doc = f"{platform}: cst[{cst}].key `{key}` <-> hub:{construct}.{field} ({shape})"
    rank = 50
    has_ref = f"hub:has_{ref_kind}" if ref_kind else None

    # shared L head: the construct mapping MC and the key-entry S.
    head_n = [
        n("MC", "cst:Mapping", lit("construct", cst)),
        n("S", "cst:MappingEntry", lit("key", key)),
    ]
    head_e = [e("cst:has_child", "MC", "S")]
    # The construct (hubC) is ESTABLISHED by its own identity rule; a
    # field rule only ATTACHES to it. Under rc8's corr-rooted identity +
    # both-direction dedup this MUST be a References (context) corr — the
    # canonical idiom (cf. fase2019 Sub-/Leaf-Rule: parent corr References,
    # child corr Establishes). Re-Establishing hubC here makes every field
    # rule re-mint the construct, which rc8's dedup collapses such that the
    # `has_attr` satellite edge is lost forward. SPANS stay for reverse-
    # findability; role=References classifies it as context, not creation.
    shared = corr("MC", "hubC", SPANS, role="References")

    # Pin the hub:attr.name on the satellite. Forward rules set
    # hub:attr.name = parent_key of the cst:Scalar = the platform's
    # mapping-entry key (`script`, `before_script`, …), not the IR
    # field name. This constraint matches that. Without it, every
    # R_<plat>_<C>_<field>_<key> rule's reversed form bound any
    # hub:attr child of the construct, fabricating spurious entries
    # for unrelated fields whose names happened to share a value
    # with a present attr (seed had `name=build` → reverse emitted
    # `allow_failure: build` on every existing job attr).
    attr_with_name = ["a", "hub:attr", lit("name", key)]

    if shape in ("scalar_attr", "block_attr"):
        # UNIFIED hub:value topology (proven: unified_scalar_attr_roundtrips
        # in unified_field_model.rs, both directions green). A bare scalar
        # (or opaque block) field:
        #
        #   L: MC -has_child-> S -value_of-> SC
        #   R: hubC -has_attr-> attr{name=field, prov_key=key}
        #        -has_value-> val(hub:value)
        #   corr: MC↔hubC, S↔attr, SC↔val
        #
        # The S↔attr corr materialises the field's MappingEntry on the
        # backward pass — its ABSENCE in the old form (which corr'd only
        # SC↔a, no entry anchor) is why scalar fields lost their wrapper and
        # bug2's inner artifact fields vanished. The leaf text rides its own
        # hub:value node. name=field keeps the IR field name; prov_key=key
        # records provenance AND keeps the GhostId distinct when several
        # platform keys map to one IR field (e.g. on_new_commit/
        # on_job_failure → cancel_in_progress) — without it those collapse
        # N→1. block has no text (a cst:Mapping value): spans only.
        if shape == "scalar_attr":
            # UNIFIED hub:value form (proven: unified_scalar_attr_roundtrips
            # in unified_field_model.rs, both directions green). Identical to
            # block_attr below, but the value is a cst:Scalar leaf whose
            # `text` rides the hub:value node.
            #
            #   L: MC -has_child-> S(key=…) -value_of-> SC(Scalar)
            #   R: hubC -has_attr-> attr{name=field, prov_key=key}
            #        -has_value-> val(hub:value)
            #   corr: MC↔hubC (References), S↔attr, SC↔val (text)
            #
            # KEY-GATED via the S(MappingEntry key=…) head — this is what
            # makes sibling keys that map to ONE IR field (e.g. buildkite
            # command/commands → run, azure script/bash/pwsh → run) mutually
            # exclusive. The earlier first-class bare-MC form (L=[MC] only,
            # value lifted onto MC.<key>) had NO key gate: every sibling-key
            # rule matched every construct unconditionally and propagated onto
            # the same hubC.<field>, so when only one key was present the
            # other(s) overwrote it with the absent value — competing writes
            # that oscillate forever (buildkite step `command:` never
            # converges; proven by bisection in all_platforms_cascade).
            # Bijectivity forbids a gate-only node (compile.rs:563 — every
            # created node needs an Establishes corr; in the Bwd direction a
            # corr-less gate node is a CreatedNodeWithoutCorrespondence), so
            # the ONLY key-gated bijective scalar_attr is this structural
            # form. prov_key records which platform key was used (backward
            # emit) and keeps the GhostId distinct across sibling keys.
            # No lift needed — the rule reads the CST scalar directly.
            #
            # vkind=scalar discriminates this rule from block_attr in the
            # BACKWARD direction. A union `scalar | block` field (e.g. gitlab
            # cache.key) generates BOTH rules; they share an identical hub
            # R-pattern (hubC→attr→val), so without a discriminator both match
            # the same hub:attr backward and compete to rebuild the CST value
            # (Scalar vs Mapping) — the scalar text is lost. The literal rides
            # the ghost hash forward and gates the match backward (same idiom
            # as prov_key). scalar|list needs none — coll vs val already
            # separates seq_attr structurally.
            return [rule(
                name, rank, doc,
                head_n + [n("SC", "cst:Scalar")],
                head_e + [e("cst:value_of", "S", "SC")],
                [
                    n("hubC", f"hub:{construct}"),
                    n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
                      lit("vkind", "scalar")),
                    n("val", "hub:value"),
                ],
                [
                    e("hub:has_attr", "hubC", "attr"),
                    e("hub:has_value", "attr", "val"),
                ],
                [
                    shared,
                    corr("S", "attr", SPANS),
                    corr("SC", "val", [["text", "text"], *SPANS]),
                ],
            )]
        # block_attr: the value is a nested cst:Mapping (an opaque object —
        # `artifact_store: {location, type}`, `defaults: {shell, …}`,
        # buildkite `waiter: {}`). DECOMPOSED into two rules so an EMPTY block
        # still records its presence: the single-rule form required an inner
        # ME→MSC, so an empty mapping created NOTHING and the field's identity
        # was lost (buildkite empty gate steps collapsed to indistinguishable
        # empty hub:steps). Rule A creates the field attr + collection from the
        # block mapping ALONE (fires once, even when MM is empty); Rule B adds
        # one item per inner key→value entry, REFERENCING A's attr/coll (the
        # proven map_nodes-ref parent/item idiom: parent corr References, child
        # Establishes). Non-empty: A makes the deduped attr/coll, B adds items
        # — identical hub shape to the old rule. vkind=block keeps it disjoint
        # from scalar_attr under a scalar|block union. (Pairs with the seeder
        # normalising a `{}`-scalar field value to an empty cst:Mapping.)
        block_r = [
            n("hubC", f"hub:{construct}"),
            n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
              lit("vkind", "block")),
            n("coll", "hub:collection"),
        ]
        rule_a = rule(
            name, rank, f"{doc} [presence]",
            head_n + [n("MM", "cst:Mapping")],
            head_e + [e("cst:value_of", "S", "MM")],
            block_r,
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("MM", "coll", SPANS),
            ],
        )
        rule_b = rule(
            f"{name}_entry", rank, f"{doc} [entry]",
            head_n + [
                n("MM", "cst:Mapping"),
                n("ME", "cst:MappingEntry"),
                n("MSC", "cst:Scalar"),
            ],
            head_e + [
                e("cst:value_of", "S", "MM"),
                e("cst:has_child", "MM", "ME"),
                e("cst:value_of", "ME", "MSC"),
            ],
            block_r + [
                n("a", "hub:attr"),
                n("v", "hub:value"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "a"),
                e("hub:has_value", "a", "v"),
            ],
            [
                shared,
                corr("S", "attr", SPANS, role="References"),
                corr("MM", "coll", SPANS, role="References"),
                corr("ME", "a", [["key", "name"], *SPANS]),
                corr("MSC", "v", [["text", "text"], *SPANS]),
            ],
        )
        return [rule_a, rule_b]

    if shape == "seq_attr":
        # UNIFIED bijective value topology (proven: unified_seq_attr_
        # roundtrips). A scalar list's CST is SEQ→IT→SC — TWO nodes per
        # element — so each element maps to TWO hub:value nodes (the corr
        # law gives every cst node its own anchor; collapsing both onto one
        # hub:value is the forbidden N→1 creation, measured not to fire):
        #
        #   L: MC -has_child-> S -value_of-> SEQ -has_child-> IT -value_of-> SC
        #   R: hubC -has_attr-> attr{name=field, prov_key=key} -has_value-> coll
        #        coll -has_item-> rk(hub:value, item slot)
        #          -has_value-> leaf(hub:value, text)
        #   corr: MC↔hubC, S↔attr, SEQ↔coll, IT↔rk, SC↔leaf
        #
        # Same shape as seq_scalar_nodes (script); rk/leaf replace step/attr.
        ln = head_n + [
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("SC", "cst:Scalar"),
        ]
        le = head_e + [
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "SC"),
        ]
        return [rule(
            name, rank, doc, ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key)),
                n("coll", "hub:collection"),
                n("rk", "hub:value"),
                n("leaf", "hub:value"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "rk"),
                e("hub:has_value", "rk", "leaf"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("SEQ", "coll", SPANS),
                corr("IT", "rk", SPANS),
                corr("SC", "leaf", [["text", "text"], *SPANS]),
            ],
        )]

    if shape == "seq_scalar_nodes":
        # UNIFIED field-value topology (proven in unified_field_model.rs):
        # the field is a hub:attr{name=<field>} whose value is a
        # hub:collection; each list item is a fresh sub-construct (rk)
        # with its leaf text on its own hub:attr. Every cst node has a
        # 1:1 hub pendant — no corr-less skeleton, no fan-on-hubC.
        #
        #   L: MC -has_child-> S -value_of-> SEQ -has_child-> IT -value_of-> SC
        #   R: hubC -has_attr-> attr{name=field} -has_value-> coll
        #        coll -has_item-> rk -has_attr-> a{name=key}
        #   corr: MC↔hubC, S↔attr, SEQ↔coll, IT↔rk, SC↔a
        ln = head_n + [
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("SC", "cst:Scalar"),
        ]
        le = head_e + [
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "SC"),
        ]
        return [rule(
            name, rank, doc, ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key)),
                n("coll", "hub:collection"),
                n("rk", f"hub:{ref_kind}"),
                n(*attr_with_name),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "rk"),
                e("hub:has_attr", "rk", "a"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("SEQ", "coll", SPANS),
                corr("IT", "rk", SPANS),
                corr("SC", "a", ATTR_BINDS),
            ],
        )]

    if shape == "seq_block_attr":
        # A NON-ref list whose ITEMS are OBJECTS (list<map<string>>,
        # list<Volume>, list<selectField|textField>, …) folded into an
        # aggregation field (step.env, job.options, step.gate, hook.phase, …).
        # The plain seq_attr above expects scalar items (IT->SC) and so never
        # matches an object item (IT->Mapping) → the whole field is dropped.
        # Capture each object item as a concrete hub:item carrying its key→value
        # entries (one level): the item is the per-element node (IT↔item); each
        # of the object's entries rides an a{name=key}->value leaf.
        #
        #   L: MC -child-> S -value_of-> SEQ -child-> IT -value_of-> IM(Mapping)
        #        IM -child-> IME -value_of-> IMSC(Scalar)
        #   R: hubC -has_attr-> attr{name=field,vkind=seqblock} -has_value-> coll
        #        coll -has_item-> item -has_attr-> a{name=<key>} -has_value-> v
        #   corr: MC↔hubC(Ref), S↔attr, SEQ↔coll, IT↔item, IME↔a(key→name),
        #         IMSC↔v(text). Fires once per (item, entry); the shared IT↔item
        #         corr keeps all of one object's entries on the same item.
        ln = head_n + [
            n("SEQ", "cst:Sequence"),
            n("IT", "cst:SequenceItem"),
            n("IM", "cst:Mapping"),
            n("IME", "cst:MappingEntry"),
            n("IMSC", "cst:Scalar"),
        ]
        le = head_e + [
            e("cst:value_of", "S", "SEQ"),
            e("cst:has_child", "SEQ", "IT"),
            e("cst:value_of", "IT", "IM"),
            e("cst:has_child", "IM", "IME"),
            e("cst:value_of", "IME", "IMSC"),
        ]
        block_rule = rule(
            _ident(f"{name}_block"), rank, doc + " [block item]", ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
                  lit("vkind", "seqblock")),
                n("coll", "hub:collection"),
                n("item", "hub:item", lit("vkind", "block")),
                n("a", "hub:attr"),
                n("v", "hub:value"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "item"),
                e("hub:has_attr", "item", "a"),
                e("hub:has_value", "a", "v"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("SEQ", "coll", SPANS),
                corr("IT", "item", SPANS),
                # IM (the object mapping) and IT (the SequenceItem) are TWO cst
                # nodes for the ONE hub:item; both must carry a corr (rc8
                # created-node invariant). Anchor IM on item too, with a
                # DISTINCT corr kind so its backward-created GhostId doesn't
                # collide with IT↔item.
                corr("IM", "item", SPANS, kind="seqblock_wrap"),
                corr("IME", "a", [["key", "name"], *SPANS]),
                corr("IMSC", "v", [["text", "text"], *SPANS]),
            ],
        )
        # SCALAR arm: object-lists routinely also contain bare scalars and empty
        # `{}` (which the cst parser renders as a FlowMap *Scalar*, not a
        # Mapping). vkind=scalar discriminates it from the block arm backward.
        scalar_rule = rule(
            _ident(f"{name}_scalar"), rank, doc + " [scalar item]",
            head_n + [n("SEQ", "cst:Sequence"), n("IT", "cst:SequenceItem"), n("SC", "cst:Scalar")],
            head_e + [e("cst:value_of", "S", "SEQ"), e("cst:has_child", "SEQ", "IT"),
                      e("cst:value_of", "IT", "SC")],
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
                  lit("vkind", "seqblock")),
                n("coll", "hub:collection"),
                n("item", "hub:item", lit("vkind", "scalar")),
                n("v", "hub:value"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "item"),
                e("hub:item_value", "item", "v"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("SEQ", "coll", SPANS),
                corr("IT", "item", SPANS),
                corr("SC", "v", [["text", "text"], *SPANS]),
            ],
        )
        return [block_rule, scalar_rule]

    if shape == "scalar_node":
        # UNIFIED topology, single-value variant of seq_scalar_nodes: a
        # bare scalar under the field key (no cst:Sequence wrapper) is
        # still an IR collection of one element. Same hub shape as the seq
        # case; SC is BOTH the per-item node (anchors rk) and the leaf
        # (anchors a) — there is no IT, so SC carries both roles.
        #
        #   L: MC -has_child-> S -value_of-> SC
        #   R: hubC -has_attr-> attr{name=field} -has_value-> coll
        #        coll -has_item-> rk{name=<scalar>}   (construct, FIRST-CLASS name)
        #   corr: MC↔hubC, S↔attr, S↔coll, SC↔rk (text→name)
        #
        # rc8/first-class re-arch: the OLD form gave SC a DOUBLE corr —
        # SC↔rk (the item construct) AND SC↔a (a separate leaf satellite) —
        # i.e. one cst:Scalar created TWO hub nodes. That dual role tangled
        # the backward cst (MappingEntry→value_of→MappingEntry etc., bug4).
        # Fix: drop the leaf `a`; the scalar value rides FIRST-CLASS on the
        # construct as `rk.name` via the SC↔rk binding text→name. One cst
        # node → one hub node. emit renders rk by form (name-only construct
        # → inline scalar). `coll` is correct for a list-of-bare-scalar field
        # and a tolerable single-element wrapper for a single ref; anchored
        # on S (no cst:Sequence here). prov/key stays on the field-attr.
        ln = head_n + [n("SC", "cst:Scalar")]
        le = head_e + [e("cst:value_of", "S", "SC")]
        return [rule(
            name, rank, doc, ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key)),
                n("coll", "hub:collection"),
                n("rk", f"hub:{ref_kind}"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "rk"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("S", "coll", SPANS),
                corr("SC", "rk", [["text", "name"], *SPANS]),
            ],
        )]

    if shape in ("mapping_node", "seq_mapping_nodes"):
        # UNIFIED topology, ref-element variant: the value is a mapping
        # (or list of mappings) that the seeder tags as a sub-construct.
        # The field is hub:attr{name=field} -has_value-> hub:collection;
        # each element IS the construct (IM↔rk directly — no leaf attr,
        # the element's own field rules populate it). Every cst node 1:1:
        #
        #   seq:  L MC->S->SEQ->IT->IM(construct)
        #         R hubC -has_attr-> attr -has_value-> coll -has_item-> rk
        #         corr MC↔hubC, S↔attr, SEQ↔coll, IT↔rk, IM↔rk?
        #   single: L MC->S->IM(construct)
        #         R hubC -has_attr-> attr -has_value-> coll -has_item-> rk
        #         corr MC↔hubC, S↔attr, IM↔rk (no SEQ; coll has one item)
        #
        # NOTE on the seq case: IT (SequenceItem) and IM (the construct
        # mapping) are TWO cst nodes but map to ONE hub element (rk). Per
        # the cardinality law a hub node has exactly one creator anchor —
        # so IT↔rk (the per-item slot, like the proven script case) is the
        # creator, and IM is the construct mapping the element's own
        # construct_rule already corr's to rk. Here IT↔rk creates; IM is
        # context (its construct_rule owns it). The single case has no IT,
        # so IM↔rk is the creator there.
        #
        # rc8 created-node invariant: the context intent for IM must be an
        # explicit `References` corr (IM↔rk) — otherwise IM has NO corr and
        # the reverse direction creates it anchorless. References carries
        # SPANS (rc7+) for reverse-findability but stays context, so IM is
        # matched, not minted (no ghost-twin of the construct's own rule).
        if shape == "seq_mapping_nodes":
            # LIST of constructs: hubC -has_attr[field]-> attr -has_value->
            # coll -has_item-> rk (per item). The cst Sequence SEQ↔coll gives
            # the collection its own anchor (NOT on S) — the per-item slot IT
            # anchors rk; IM is the construct mapping owned by construct_rule.
            ln = head_n + [
                n("SEQ", "cst:Sequence"),
                n("IT", "cst:SequenceItem"),
                n("IM", "cst:Mapping", lit("construct", ref_kind)),
            ]
            le = head_e + [
                e("cst:value_of", "S", "SEQ"),
                e("cst:has_child", "SEQ", "IT"),
                e("cst:value_of", "IT", "IM"),
            ]
            return [rule(
                name, rank, doc, ln, le,
                [
                    n("hubC", f"hub:{construct}"),
                    # vkind=seq discriminates the LIST arm of a `list<X> |
                    # map<X>` union from the map_nodes arm in the BACKWARD
                    # direction: both produce hubC -has_attr-> coll -has_item->
                    # rk, so without it both match the same hub and compete to
                    # rebuild the CST (a SequenceItem vs a MappingEntry),
                    # scrambling step content (woodpecker `steps:`).
                    n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
                      lit("vkind", "seq")),
                    n("coll", "hub:collection"),
                    n("rk", f"hub:{ref_kind}"),
                ],
                [
                    e("hub:has_attr", "hubC", "attr"),
                    e("hub:has_value", "attr", "coll"),
                    e("hub:has_item", "coll", "rk"),
                ],
                [
                    shared,
                    corr("S", "attr", SPANS),
                    corr("SEQ", "coll", SPANS),
                    corr("IT", "rk", SPANS),
                    corr("IM", "rk", SPANS, role="References"),
                ],
            )]
        # SINGLE construct (mapping_node): ONE instance, NO collection. The
        # value is hubC -has_attr[field]-> attr -has_value-> rk (the construct
        # DIRECTLY) — no spurious one-item collection, hence no intermediate
        # cst node for it. S anchors on attr (bidirectional, exactly like
        # map_nodes' field entry); IM is the construct mapping owned by
        # construct_rule (References). This eliminates the old dual S↔attr +
        # S↔coll anchor (the coll-on-S workaround that had no cst pendant): on
        # reverse that minted TWO distinct S nodes (one per anchor) and
        # collapsed every single ref construct (concurrency, permissions,
        # artifacts, single stage). One construct → one entry → one S.
        ln = head_n + [n("IM", "cst:Mapping", lit("construct", ref_kind))]
        le = head_e + [e("cst:value_of", "S", "IM")]
        return [rule(
            name, rank, doc, ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key)),
                n("rk", f"hub:{ref_kind}"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "rk"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("IM", "rk", SPANS, role="References"),
            ],
        )]

    if shape == "map_nodes":
        if ref_kind:
            # map<X> where X is a ref: each child entry's value-
            # mapping IS an instance of the referenced construct.
            # Tag-driven: the seeder marks the inner mapping with
            # construct=<ref_kind>; this rule then links hubC →
            # hub:<ref_kind>. The inner mapping's `name` (= the
            # entry key) lives on a hub:attr satellite the
            # name-carrier path already populates.
            ln = head_n + [
                n("MM", "cst:Mapping"),
                n("ME", "cst:MappingEntry"),
                n("IM", "cst:Mapping", lit("construct", ref_kind)),
            ]
            le = head_e + [
                e("cst:value_of", "S", "MM"),
                e("cst:has_child", "MM", "ME"),
                e("cst:value_of", "ME", "IM"),
            ]
            # attr+coll form (Option A) — anchors EVERY cst node so the
            # rc8 created-node invariant holds in both directions:
            #   S (field entry `parameters:`) ↔ attr{name=field}
            #   MM (the map mapping)          ↔ coll  (the map IS the list)
            #   ME (name entry `build:`)      ↔ a{name=name} via key↔value
            #   IM (the construct mapping)    ↔ rk    (References: the
            #                                   construct's own rule owns it)
            # The ME↔name binding applies to ALL ref kinds, not just
            # job/pipeline: in a map<ref> the entry key IS the construct's
            # identity (a parameter/agent map is keyed by its name too).
            # Otherwise reverse emits empty-key entries and the names are
            # lost. SPANS make S/MM/ME creation-R anchors in reverse.
            return [rule(
                name, rank, doc, ln, le,
                [
                    n("hubC", f"hub:{construct}"),
                    # vkind=map: the MAP arm's counterpart to seq_mapping_nodes'
                    # vkind=seq — disjoint backward under a list<X>|map<X> union.
                    n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
                      lit("vkind", "map")),
                    n("coll", "hub:collection"),
                    n("rk", f"hub:{ref_kind}"),
                    n("a", "hub:attr", lit("name", "name")),
                ],
                [
                    e("hub:has_attr", "hubC", "attr"),
                    e("hub:has_value", "attr", "coll"),
                    e("hub:has_item", "coll", "rk"),
                    e("hub:has_attr", "rk", "a"),
                ],
                [
                    shared,
                    corr("S", "attr", SPANS),
                    corr("MM", "coll", SPANS),
                    corr("ME", "a", [["key", "value"]]),
                    corr("IM", "rk", SPANS, role="References"),
                ],
            )]
        # map<X> where X is a scalar leaf (e.g. `environment: {VAR: val}`):
        # an attr+coll field whose items are key/value satellites. DEEP ROOT
        # (Sandra's anchor + IR-reduction causes): the OLD form flattened the
        # map into per-entry hub:attr satellites DIRECTLY on the construct
        # (discriminated by a `target_field` literal) — the outer map wrapper
        # had NO hub node, so the field-entry S and the map MM were
        # uncorresponded → backward-orphaned (rc8 created-node violation).
        # Fix: the source map MM IS the collection (MM↔coll), the field-entry
        # S anchors the field-attr (S↔attr); each inner entry is one item
        # `a{name=<key>}` whose value rides a hub:value leaf — ME↔a (key→name),
        # MSC↔v (text). One cst node → one hub node throughout (no dual-corr).
        ln = head_n + [
            n("MM", "cst:Mapping"),
            n("ME", "cst:MappingEntry"),
            n("MSC", "cst:Scalar"),
        ]
        le = head_e + [
            e("cst:value_of", "S", "MM"),
            e("cst:has_child", "MM", "ME"),
            e("cst:value_of", "ME", "MSC"),
        ]
        return [rule(
            name, rank, doc, ln, le,
            [
                n("hubC", f"hub:{construct}"),
                n("attr", "hub:attr", lit("name", field), lit("prov_key", key)),
                n("coll", "hub:collection"),
                n("a", "hub:attr"),
                n("v", "hub:value"),
            ],
            [
                e("hub:has_attr", "hubC", "attr"),
                e("hub:has_value", "attr", "coll"),
                e("hub:has_item", "coll", "a"),
                e("hub:has_value", "a", "v"),
            ],
            [
                shared,
                corr("S", "attr", SPANS),
                corr("MM", "coll", SPANS),
                corr("ME", "a", [["key", "name"], *SPANS]),
                corr("MSC", "v", [["text", "text"], *SPANS]),
            ],
        )]

    return []  # unknown shape — skip


def _top_split(t, sep):
    """Split on `sep` only at angle-bracket depth 0."""
    parts, depth, cur = [], 0, ""
    for ch in t:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        if ch == sep and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    parts.append(cur.strip())
    return parts


def expand_union(type_str, ir_field_is_ref):
    """A union type -> the concrete shapes of its arms.

    `list<X>` arms split by what X is: a scalar-ish X means the
    sequence contains scalars (`seq_scalar_nodes` when the IR
    field is a ref, `seq_attr` otherwise); a non-scalarish X (an
    object or another union) means the sequence contains nested
    mappings (`seq_mapping_nodes` when ref, `seq_attr` for plain
    aggregations). Without this distinction, `list<object>` arms
    lowered to `seq_scalar_nodes` and the field rule expected a
    cst:Scalar where the seeder had placed a cst:Mapping.
    """
    shapes = []
    for arm in _top_split(type_str, "|"):
        is_list = arm.startswith("list<")
        is_map = arm.startswith("map<")
        if is_map:
            # A `map<X>` arm = a MAP of named constructs (woodpecker
            # `steps: {name: step}`, or any `list<X> | map<X>` union).
            # It lowers to `map_nodes` (name-keyed containment), NOT
            # `mapping_node` (a single object) — the previous fall-through
            # treated the map form as one construct and collapsed an
            # N-entry map to one on reverse. For a non-ref map of scalars
            # it's the env-style `map_nodes` (handled non-ref in field_rule).
            shapes.append("map_nodes")
        elif is_list:
            inner = arm[5:-1] if arm.endswith(">") else arm[5:]
            # If ANY arm of the inner union is a leaf scalar, the
            # outer list can ultimately materialise as a list of
            # scalars (a `gitlab.script:` is `list<list<string> |
            # string>` — items are command strings either way).
            # If EVERY arm is structural (object / nested list /
            # union of those), the list contains mappings.
            inner_arms = _top_split(inner, "|")
            any_scalar = any(_scalarish(a) for a in inner_arms)
            if ir_field_is_ref:
                shapes.append(
                    "seq_scalar_nodes" if any_scalar else "seq_mapping_nodes"
                )
            else:
                shapes.append("seq_attr")
        elif ir_field_is_ref:
            # A ref field's SCALAR arm is a single scalar value (e.g.
            # `image: gamma`). DEEP ROOT (Sandra's cause #3, over-reduction):
            # modelling it as a hub CONSTRUCT (scalar_node → hub:image) over-
            # reduces a plain scalar into a construct-collection AND creates
            # the union backward conflict — the scalar arm's hub:image and
            # the mapping arm's hub:image are the SAME kind, so both arms
            # reconstruct the same node backward and tangle. Model it
            # FIRST-CLASS (scalar_attr → hub:<parent>.<field> = the scalar)
            # instead: no construct, no shared hub:image, no backward
            # conflict. The MAPPING arm stays mapping_node (a real construct);
            # the two arms now produce DISJOINT hub shapes (scalar attr vs
            # construct), so backward only the matching arm fires.
            shapes.append("mapping_node" if not _scalarish(arm) else "scalar_attr")
        else:
            shapes.append("block_attr" if not _scalarish(arm) else "scalar_attr")
    # dedupe, keep order
    return list(dict.fromkeys(shapes))


def _scalarish(t):
    # `any` is intentionally NOT considered scalar: it shows up on
    # platforms whose schema is loose (drone steps are typed
    # `list<any>`) and the value in practice is always a structured
    # object. Treating it as scalar made the union expansion pick
    # `seq_scalar_nodes`, where the seeder had tagged mappings.
    return t in {"string", "boolean", "integer", "number", "enum", "null"}


def is_scalar_map(type_str):
    """True for `map<string>` / `map<bool>` … — a map whose VALUES are bare
    scalars (`{KEY: value}`). Such a field models key→value pairs, NOT a map of
    construct objects, even when the IR field points at a ref construct
    (gitlab `variables: map<string>` → variable, `inputs: map<string>` →
    parameter): the value `beta` is the variable's VALUE, not a `{…}` object.
    The map_nodes REF form (which matches `{KEY: {…object…}}`) never matches
    `{KEY: scalar}` and drops the content — use the non-ref scalar form."""
    t = type_str.strip()
    if t.startswith("map<") and t.endswith(">"):
        inner = t[4:-1]
        arms = _top_split(inner, "|")
        return arms and all(_scalarish(a) for a in arms)
    return False


def is_object_list(type_str):
    """True for `list<object…>` whose items are ALL non-scalar (objects /
    maps), e.g. `list<map<string>>`, `list<Volume>`, `list<selectField |
    textField>`. Such a list folded into a NON-ref aggregation field is lowered
    to seq_attr (scalar items) and drops the object content — it needs the
    seq_block_attr form instead."""
    t = type_str.strip()
    if not t.startswith("list<"):
        return False
    inner = t[5:t.rfind(">")] if ">" in t else t[5:]
    arms = _top_split(inner, "|")
    return bool(arms) and all(not _scalarish(a) for a in arms)


def is_mixed_seq_union(type_str):
    """True for a `list<…object… | …scalar…>` whose items are EITHER a
    construct mapping OR a bare scalar (gitlab `services: list<map<string> |
    string>`, buildkite `steps: list<commandStep | … | enum>`). Such a list
    cannot be a single seq_mapping_nodes (scalar items get no rule and are
    dropped) nor a seq_scalar_nodes (object items mis-modelled), and the naive
    both-arms form tangles backward. It needs the concrete-item wrapper."""
    for arm in _top_split(type_str, "|"):
        if not arm.startswith("list<"):
            continue
        inner = arm[5:-1] if arm.endswith(">") else arm[5:]
        ia = _top_split(inner, "|")
        has_scalar = any(_scalarish(a) for a in ia)
        has_obj = any(not _scalarish(a) for a in ia)  # object / map<…> / nested
        if has_scalar and has_obj:
            return True
    return False


def seq_item_union_rules(platform, construct, field, key, ref_kind,
                         cst_construct=None, child_cst_construct=None,
                         child_has_construct_rule=False):
    """Mixed `list<construct | scalar>` → a concrete per-element `hub:item`
    node (1:1 with the cst:SequenceItem) that CARRIES the variant:

        hub:<C> -has_attr-> attr{name=field,vkind=seqitem} -has_value-> coll
        coll -has_item-> item{vkind=mapping} -item_element-> hub:<ref_kind>
        coll -has_item-> item{vkind=scalar}  -item_value->   hub:value

    The `vkind` on the item discriminates the two arms BACKWARD, so they no
    longer compete for `coll -has_item-> rk` (the failure of the naive both-arms
    form). The item is the node the FIELD rule owns (anchored on IT); the
    construct (rk) stays owned by its construct rule (References). Two rules,
    one per arm, sharing hubC/attr/coll (via MC/S/SEQ corrs)."""
    cst = cst_construct or construct
    child_cst = child_cst_construct or ref_kind
    head_n = [
        n("MC", "cst:Mapping", lit("construct", cst)),
        n("S", "cst:MappingEntry", lit("key", key)),
        n("SEQ", "cst:Sequence"),
        n("IT", "cst:SequenceItem"),
    ]
    head_e = [
        e("cst:has_child", "MC", "S"),
        e("cst:value_of", "S", "SEQ"),
        e("cst:has_child", "SEQ", "IT"),
    ]
    shared_n = [
        n("hubC", f"hub:{construct}"),
        n("attr", "hub:attr", lit("name", field), lit("prov_key", key),
          lit("vkind", "seqitem")),
        n("coll", "hub:collection"),
    ]
    shared_e = [
        e("hub:has_attr", "hubC", "attr"),
        e("hub:has_value", "attr", "coll"),
    ]
    shared_corr = [
        corr("MC", "hubC", SPANS, role="References"),
        corr("S", "attr", SPANS),
        corr("SEQ", "coll", SPANS),
    ]
    map_rule = rule(
        _ident(f"R_{platform}_{construct}_{field}_{key}_item_map"),
        50,
        f"{platform}: cst[{cst}].`{key}` mixed-list MAPPING item <-> hub:{construct}.{field} item->element->{ref_kind}",
        head_n + [n("IM", "cst:Mapping", lit("construct", child_cst))],
        head_e + [e("cst:value_of", "IT", "IM")],
        shared_n + [
            n("item", "hub:item", lit("vkind", "mapping")),
            n("rk", f"hub:{ref_kind}"),
        ],
        shared_e + [
            e("hub:has_item", "coll", "item"),
            e("hub:item_element", "item", "rk"),
        ],
        shared_corr + [
            corr("IT", "item", SPANS),
            # IT↔item and IM↔rk are TWO distinct cst anchors (SequenceItem vs
            # Mapping) for TWO distinct hub nodes — no fan-in. rk is owned by
            # the construct's own rule (References). NOTE: when the construct has
            # NO rule (gitlab `service` = generic map<string>), Establishing rk
            # here captures the map item but its content stays empty → backward
            # asymmetry (regresses gitlab/travis). Kept as References (item-map
            # then only fires where a construct rule exists); the scalar arm and
            # construct-backed mapping arms still work.
            corr("IM", "rk", SPANS, role="References"),
        ],
    )
    _ = child_has_construct_rule
    scalar_rule = rule(
        _ident(f"R_{platform}_{construct}_{field}_{key}_item_scalar"),
        50,
        f"{platform}: cst[{cst}].`{key}` mixed-list SCALAR item <-> hub:{construct}.{field} item->value",
        head_n + [n("SC", "cst:Scalar")],
        head_e + [e("cst:value_of", "IT", "SC")],
        shared_n + [
            n("item", "hub:item", lit("vkind", "scalar")),
            n("val", "hub:value"),
        ],
        shared_e + [
            e("hub:has_item", "coll", "item"),
            e("hub:item_value", "item", "val"),
        ],
        shared_corr + [
            corr("IT", "item", SPANS),
            corr("SC", "val", [["text", "text"], *SPANS]),
        ],
    )
    return [map_rule, scalar_rule]


def concept_rule(platform, concept_name, concept, dotted_path):
    """Build a TGG rule from a semantic concept + per-platform path.

    Each platform path (e.g. drone's `trigger.branch.include`,
    github's `on.push.branches`) walks a CST chain to a list of
    scalar leaves. The concept declares the satellite name to
    attach to `hub:<parent>` — every leaf becomes a hub:attr
    under hub:<parent> with name=<satellite_name>, value=<leaf>.

    By pinning a fixed satellite_name across platforms, drone and
    github both produce `hub:pipeline.has_attr → hub:attr[name=
    trigger_branch, value=main]` — same hub-subgraph, syntactic
    variation handled entirely by the per-platform L-pattern.
    """
    # The hub-side parent is fixed (one concept → one hub-construct);
    # the CST-side parent may differ per platform because each
    # platform uses its own construct name for the step-like node
    # (drone: "step", buildkite: "commandStep", argo: "workflow_step",
    # google_cloudbuild: "BuildStep", azure: "task", ...). The
    # optional `platform_parent` table per concept maps the platform
    # to the CST construct name; otherwise we fall back to `parent`.
    hub_parent = concept["parent"]
    cst_parent = concept.get("platform_parent", {}).get(platform, hub_parent)
    # `satellite_name` is required only for the legacy satellite
    # mode; IR-extending concepts (with `target_construct`) bind
    # the leaf to a real field on the target node instead.
    satellite_name = concept.get("satellite_name")
    # `leaf_shape` = "list" (default; final entry's value is a
    # cst:Sequence of cst:Scalar items) or "scalar" (final entry's
    # value is a single cst:Scalar). List-shape fires once per
    # item; scalar-shape fires once per match.
    leaf_shape = concept.get("leaf_shape", "list")

    segments = dotted_path.split(".")
    # `key[]` can never be a leaf (the value under `key:` is a
    # sequence, not a scalar). `<>` as a leaf IS valid — it means
    # "for each entry in the surrounding map, the entry's value is
    # the scalar leaf" (e.g. `env.variables.<>` extracts every
    # value in an env-var map).
    if segments[-1].endswith("[]"):
        return None

    # L-pattern: cst:Mapping[construct=parent] -> chain.
    #
    # Each segment except the last is one of:
    #   * `key`   — ME[key=key] -value_of-> Mapping[parent_key=key]; next anchor = that Mapping
    #   * `key[]` — ME[key=key] -value_of-> Sequence -has_child-> SequenceItem
    #               -value_of-> Mapping; next anchor = that inner Mapping. Fires once per item.
    #   * `<>`    — anonymous map-entry iteration. ME(any key) -value_of-> Mapping;
    #               next anchor = that inner Mapping. Fires once per map entry,
    #               regardless of the entry's key name. Used for paths whose
    #               intermediate key is the *value* (aws_codebuild
    #               `phases.<>.commands` — phase name carries semantic
    #               weight but the concept rule treats them uniformly).
    l_nodes = [n("MC", "cst:Mapping", lit("construct", cst_parent))]
    l_edges = []
    prev_map = "MC"
    # The leaf's `cst:MappingEntry` is the natural anchor for the
    # IR-extending target construct: its key constraint
    # (`branches`, `tags`, `paths`, …) differs per concept, so
    # distinct concepts get distinct hub:<target_construct>
    # GhostIds even when sibling concepts share the same enclosing
    # mapping (drone `trigger.branch.include` and `trigger.event.include`
    # both anchor at the leaf entry, not at the common
    # `M[parent_key=trigger]`). Captured during the loop below.
    leaf_entry_anchor = "E0"
    for i, raw in enumerate(segments):
        is_last = i == len(segments) - 1
        if is_last:
            leaf_entry_anchor = f"E{i}"
        if raw == "<>":
            entry_id = f"E{i}"
            l_nodes.append(n(entry_id, "cst:MappingEntry"))
            l_edges.append(e("cst:has_child", prev_map, entry_id))
            if is_last:
                # `<>` as leaf: each map entry's value is the scalar
                # leaf. `env.variables.<>` produces one match per
                # variable, with SC bound to the variable's value.
                # leaf_shape="list" would be nonsensical here (the
                # entry value is the leaf, not a sequence under it).
                l_nodes.append(n("SC", "cst:Scalar"))
                l_edges.append(e("cst:value_of", entry_id, "SC"))
            else:
                inner_id = f"IM{i + 1}"
                l_nodes.append(n(inner_id, "cst:Mapping"))
                l_edges.append(e("cst:value_of", entry_id, inner_id))
                prev_map = inner_id
            continue
        seq_iter = raw.endswith("[]")
        key = raw[:-2] if seq_iter else raw
        entry_id = f"E{i}"
        l_nodes.append(n(entry_id, "cst:MappingEntry", lit("key", key)))
        l_edges.append(e("cst:has_child", prev_map, entry_id))
        if is_last:
            if leaf_shape == "scalar":
                l_nodes.append(n("SC", "cst:Scalar"))
                l_edges.append(e("cst:value_of", entry_id, "SC"))
            else:
                l_nodes.append(n("SEQ", "cst:Sequence"))
                l_nodes.append(n("IT", "cst:SequenceItem"))
                l_nodes.append(n("SC", "cst:Scalar"))
                l_edges += [
                    e("cst:value_of", entry_id, "SEQ"),
                    e("cst:has_child", "SEQ", "IT"),
                    e("cst:value_of", "IT", "SC"),
                ]
        elif seq_iter:
            seq_id = f"SEQ{i}"
            it_id = f"IT{i}"
            inner_id = f"IM{i + 1}"
            l_nodes.append(n(seq_id, "cst:Sequence"))
            l_nodes.append(n(it_id, "cst:SequenceItem"))
            l_nodes.append(n(inner_id, "cst:Mapping"))
            l_edges += [
                e("cst:value_of", entry_id, seq_id),
                e("cst:has_child", seq_id, it_id),
                e("cst:value_of", it_id, inner_id),
            ]
            prev_map = inner_id
        else:
            map_id = f"M{i + 1}"
            l_nodes.append(n(map_id, "cst:Mapping", lit("parent_key", key)))
            l_edges.append(e("cst:value_of", entry_id, map_id))
            prev_map = map_id

    # R-pattern: two flavours.
    #
    # ── IR-extending mode ──────────────────────────────────────────
    # `target_construct` is set → emit a proper hub:<target_construct>
    # node with the leaf bound to `target_field`:
    #
    #   hub:<parent> --<edge>--> hub:<target_construct>[
    #       <discriminator_attr> = <discriminator_value>,
    #       <target_field>       = <leaf>
    #   ]
    #
    # Multiple matches produce distinct hub:<target_construct>
    # nodes because their `target_field` value differs (the GhostId
    # hash includes sorted attrs).
    #
    # ── Satellite mode (legacy) ────────────────────────────────────
    # No `target_construct` → fall back to hub:attr satellites under
    # the parent. Kept for concepts whose canonical IR field doesn't
    # exist yet (`module_sdk`, `pipeline_timeout`, …).
    target_construct = concept.get("target_construct")
    if target_construct:
        target_field = concept["target_field"]
        edge_kind = concept.get("edge", f"hub:has_{target_construct}")
        if not edge_kind.startswith("hub:"):
            edge_kind = f"hub:{edge_kind}"
        # R-pattern follows the existing IR convention: scalar
        # field values live as `hub:attr` satellites under their
        # owning construct, not as direct attributes on the node.
        # The discriminator (`kind=branch`, …) is set directly on
        # the target node so distinct concept rules can share one
        # construct without collapsing into a single GhostId.
        #
        #   hub:<parent> --<edge>--> hub:<target_construct>[disc=val]
        #       hub:<target_construct> --hub:has_attr--> hub:attr[name=<field>, value=<scalar>]
        #
        # Matches the structure azure / buildkite already emit for
        # `R_<plat>_trigger_branches_include` (hub:trigger →
        # hub:attr[name=include]), so the two paths interoperate.
        target_attrs = []
        if "discriminator_attr" in concept:
            target_attrs.append(lit(concept["discriminator_attr"], concept["discriminator_value"]))
        r_nodes = [
            n("hubP", f"hub:{hub_parent}"),
            n("hubT", f"hub:{target_construct}", *target_attrs),
            n("a", "hub:attr", lit("name", target_field)),
        ]
        r_edges = [
            e(edge_kind, "hubP", "hubT"),
            e("hub:has_attr", "hubT", "a"),
        ]
        # Each R-creation node needs an L-anchor. MC anchors hubP
        # (one pipeline). The leaf MappingEntry anchors hubT — its
        # key constraint differs per concept (`branches` vs
        # `tags` vs `paths`), so sibling concepts on the same
        # pipeline get distinct hub:<target_construct> GhostIds.
        # Multiple sequence items under the same MappingEntry
        # (e.g. `branches: [main, dev]`) all produce hub:attr
        # children of ONE hub:trigger.
        corrs = [
            corr("MC", "hubP", SPANS),
            corr(leaf_entry_anchor, "hubT", SPANS),
            corr("SC", "a", [["text", "value"], *SPANS]),
        ]
        doc = (
            f"{platform}: concept `{concept_name}` via path `{dotted_path}`"
            f" <-> hub:{hub_parent} -{edge_kind[4:]}-> hub:{target_construct}"
            f".attr[name={target_field}]"
        )
    else:
        # Legacy satellite shape — hub:attr child under hub:<parent>.
        r_nodes = [
            n("hubP", f"hub:{hub_parent}"),
            n("a", "hub:attr", lit("name", satellite_name)),
        ]
        r_edges = [e("hub:has_attr", "hubP", "a")]
        corrs = [
            corr("MC", "hubP", SPANS),
            corr("SC", "a", [["text", "value"], *SPANS]),
        ]
        doc = (
            f"{platform}: concept `{concept_name}` via path `{dotted_path}`"
            f" <-> hub:{hub_parent}.attr[name={satellite_name}]"
        )

    # ── The syntactic path-wrapper nodes are CONTEXT, not creations ──
    # The intermediate path entries/maps (E0/M1/E1/M2 …) and the leaf
    # sequence (SEQ/IT) are pure platform SYNTAX with NO hub pendant —
    # that is the whole point of concept normalisation (drone's
    # `trigger.branch.include` and github's `on.push.branches` collapse
    # onto ONE hub subgraph; the path lengths even differ per platform,
    # so the hub CANNOT store a fixed path structure). They cannot be
    # CREATED bidirectionally: normalisation reduces node count, so the
    # cst↔hub mapping is non-bijective and an Establishes corr would
    # mint a DUPLICATE hub node per wrapper (distinct corr kind ⇒
    # distinct GhostId — proven: drone got 3 hub:pipeline). Instead
    # declare them as REFERENCES onto the pipeline: rc8 routes a
    # reference-corr target into the MATCH, not nodes_to_create
    # (compile.rs §4b), so the invariant is satisfied WITHOUT minting
    # hub nodes. Forward the wrappers are matched context under the
    # (already-created) pipeline; the leaf trigger + value remain the
    # only Establishes creations. Backward denormalisation of the path
    # is emit's job (it knows platform + concept path) — the lossless
    # A→B story, not a TGG creation.
    anchored = {c["l_node_id"] for c in corrs}
    for nd in l_nodes:
        nid = nd["id"]
        if nid in anchored:
            continue
        corrs.append(corr(nid, "hubP", SPANS, role="References"))

    return rule(
        _ident(f"R_{platform}_concept_{concept_name}"),
        55,
        doc,
        l_nodes,
        l_edges,
        r_nodes,
        r_edges,
        corrs,
    )


def main():
    targets_cfg = _load("targets.toml")
    targets = [t for t in targets_cfg if t != "meta"]
    ir = _load("ir.toml")
    hub = _load("hub_schema.toml")
    # Optional: semantic concept declarations (path -> hub-subgraph).
    concepts = {}
    try:
        concepts = _load("concepts.toml").get("concept", {})
    except FileNotFoundError:
        pass
    field_kind = {
        (c, f): k
        for c, node in hub.get("node", {}).items()
        for f, k in node.get("fields", {}).items()
    }
    out_dir = CAT / "rules"
    total = 0
    summary = []

    for plat in targets:
        if not (out_dir / f"{plat}.toml").exists():
            continue
        manifest = _load(f"rules/{plat}.toml")
        constructs = sorted(
            c
            for c, node in ir.items()
            if isinstance(node, dict) and node.get("maps", {}).get(plat)
        )
        plat_constructs = set(constructs)
        # CST-alias map: for each IR construct, which CST tags the
        # seeder actually emits for it. Two sources:
        #
        #   1. The IR construct name itself — `open_pipeline` tags
        #      the root mapping with `construct=pipeline`, and most
        #      classify tables emit only canonical IR names.
        #
        #   2. The intersection of `[<ir>.maps].<platform>` with the
        #      platform's classify-table values, parsed from
        #      `crates/pipeline-tgg-seeder/src/classify/<plat>.rs`.
        #      That intersection captures the case where one CST
        #      tag is reused for multiple IR identities — drone's
        #      "step" tag is in [job.maps] AND [step.maps], so it
        #      maps to BOTH hub:job and hub:step.
        classify_tags = _read_classify_tags(plat)
        cst_aliases: dict = {}
        for c in constructs:
            aliases = set()
            # The canonical IR name is a CST alias ONLY when the classify
            # table actually emits `construct=c`. Adding it unconditionally
            # generated rules for a CST tag the seeder never produces (e.g.
            # aws_codepipeline maps hub:parameter onto cst `variable`, never
            # `parameter`): dead forward, but in REVERSE such a phantom rule
            # competes with the real alias rule on the identical hub R-pattern
            # (both rebuild hub:parameter.name) and the two oscillate forever
            # — they rebuild divergent CST constructs (parameter vs variable)
            # with no prov to disambiguate. Gate it like the declared aliases.
            if c in classify_tags:
                aliases.add(c)
            for declared in ir[c].get("maps", {}).get(plat, []):
                if declared in classify_tags:
                    aliases.add(declared)
            # Fallback: nothing matched the classify table. Keep the canonical
            # name so constructs tagged OUTSIDE the table still get rules
            # (the root `pipeline` is stamped by open_pipeline, not classify).
            if not aliases:
                aliases.add(c)
            cst_aliases[c] = sorted(aliases)

        # Find CST tags that co-create BOTH hub:job AND hub:step
        # (drone-style "step IS job" platforms). For each such tag,
        # emit job_step_link_rule to materialise the
        # hub:job -has_step-> hub:step edge — required so
        # cross-platform translation onto github's canonical
        # `pipeline → job → step` nesting works (the guiding
        # heuristic: simpler systems map onto the most
        # complex). Without this edge, drone's hub:job and
        # hub:step are sibling nodes with no structural link.
        job_cst_tags = set(cst_aliases.get("job", []))
        step_cst_tags = set(cst_aliases.get("step", []))
        co_created_cst_tags = job_cst_tags & step_cst_tags

        # CST tags the bijective wrapper rule owns. When the wrapper
        # is emitted (a flat `pipeline.<key>` seq_mapping_nodes field
        # whose child kind is a co-created step tag), it becomes the
        # SOLE authority for the pipeline->job->step nesting: it
        # creates hub:job from the SequenceItem and the two nesting
        # edges. The competing `job` construct/field/implicit rules —
        # which alias the SAME step mapping onto hub:job — must be
        # suppressed, or in reverse they write divergent CST and
        # corrupt the `jobs:` block (the v24 regression). hub:step
        # stays native (R_<plat>_step + its field rules); the wrapper
        # merely References it.
        bijective_owned_cst_tags: set = set()
        for _r in manifest.get("rule", []):
            _construct, _, _field = _r["to"].partition(".")
            _fk = field_kind.get((_construct, _field), "scalar")
            _ref_kind = _fk[4:] if _fk.startswith("ref:") else None
            if (
                _r.get("shape") == "seq_mapping_nodes"
                and _ref_kind
                and _construct == "pipeline"
                and _ref_kind in co_created_cst_tags
            ):
                bijective_owned_cst_tags.add(_ref_kind)

        # Construct + name-from-parent-key + user-comment rules:
        # one per (IR, CST-alias) pair so field rules that key on
        # the CST tag have an anchor to attach to.
        #
        # parent_key_name_rule is a FALLBACK that reads a construct's
        # name from its parent map-entry KEY ("build:" -> job.name).
        # It is correct only when the entry key IS the construct's
        # identity (name-keyed map entries / gitlab keyless top-level
        # jobs). For FIELD-entered sub-constructs the entry key is the
        # FIELD name ("artifacts:") and the real name lives in an inner
        # `name:` scalar — there the platform has a native <c>.name
        # field rule, which is the authoritative source. Emitting
        # parent_key alongside it collides on hub:<c>.name: the reverse
        # binds the field key as the name and clobbers the real value
        # (gitlab `artifacts: {name: build-${VAR}}` round-tripped to
        # `pages: {name: artifacts}`, losing the variable). So suppress
        # parent_key for any construct that has a native `<c>.name`
        # scalar rule — the inner name carries identity, not the key.
        native_name_constructs = {
            r["to"].split(".", 1)[0]
            for r in manifest.get("rule", [])
            if r.get("to", "").endswith(".name")
            and r.get("shape", "").startswith("scalar")
        }
        rules = []
        for c in constructs:
            for cst in cst_aliases[c]:
                # Suppress the `job` construct rule for tags the
                # bijective wrapper owns. The wrapper is the sole
                # creator of hub:job (anchored on the SequenceItem,
                # a distinct CST node) — keeping R_<plat>_job_from_<cst>
                # alongside re-derives hub:job from the step mapping
                # via a second path, so reverse produces two competing
                # CST anchors for the one job and corrupts output. The
                # `step` construct rule STAYS: the wrapper References
                # hub:step rather than creating it.
                if c == "job" and bijective_owned_cst_tags:
                    continue
                rules.append(construct_rule(plat, c, cst))
                if c != "pipeline" and c not in native_name_constructs:
                    rules.append(parent_key_name_rule(plat, c, cst))
        rules.extend(user_comment_rule(plat, c) for c in constructs)
        # The "bijective rule" — `nested_steps_to_jobs_rule`
        # maps a flat `steps: [...]` list onto the canonical
        # github-nested IR (pipeline -> job -> step) in one rule.
        # Emitted below in the manifest loop for any platform with a
        # pipeline-level `seq_mapping_nodes` steps field whose child
        # kind is a co-created step tag (currently drone). It is the
        # SOLE creator of hub:job; the overlapping job
        # construct/field/implicit rules are suppressed via
        # `bijective_owned_cst_tags` (see above). REVERSIBILITY came
        # from anchoring hub:job on the SequenceItem (distinct CST
        # node) instead of the step mapping — see the function
        # docstring. With that, drone reverse terminates cleanly
        # (no retraction oscillation) and drone IR is nested like
        # github's, so cross-platform drone<->github converges.
        # Collected for the carrier-suppression loop. The "name"
        # field's identity flows via parent_key_name_rule now, so
        # attr_carrier (already disabled) doesn't need a separate
        # entry, but downstream code still queries this set.
        name_covered_by_parent_key: set = {(c, "name") for c in constructs}

        # Fields where the platform's manifest already provides a
        # native carrier (map_nodes / seq_mapping_nodes / mapping_node)
        # — implicit-containment must NOT also fire for these, or in
        # reverse the cascade produces both `jobs: { build: ... }`
        # (from the field rule) AND a top-level `build:` (from the
        # implicit rule), depending on which CST anchor each rule's
        # MappingEntry lands on. circleci's roundtrip lost every job
        # this way: emit walked the implicit-created top-level
        # entry, the parser re-classified `build:` as meta, and B's
        # forward saw zero hub:jobs.
        native_ref_fields: set = set()
        for r in manifest.get("rule", []):
            shape = r.get("shape", "")
            to = r.get("to", "")
            if "." not in to:
                continue
            parent_t, field_t = to.split(".", 1)
            if shape in ("map_nodes", "mapping_node", "seq_mapping_nodes"):
                native_ref_fields.add((parent_t, field_t))
            elif shape == "union":
                # A `X | list<X>` ref union expands to mapping_node +
                # seq_mapping_nodes — those field rules ALREADY carry the
                # construct, so implicit_containment must be suppressed for
                # this (parent, field) too. Without this, a field-entered
                # sub-construct (gitlab cache: cache_item|list<cache_item>)
                # got BOTH a field rule AND a redundant implicit rule that
                # competed backward and collapsed the N-item collection to
                # one mapping. (native_ref_fields is consulted only for
                # ref:child fields, so passing is_ref=True here is safe.)
                if any(
                    s in ("mapping_node", "seq_mapping_nodes", "map_nodes")
                    for s in expand_union(r.get("type", ""), True)
                ):
                    native_ref_fields.add((parent_t, field_t))

        # Implicit containment: for every (parent, child_field=ref:child)
        # in the hub schema where both ends are constructs that the
        # platform actually emits, add a tag-only structural rule.
        # GitLab top-level jobs are the canonical case: no carrier key,
        # but the seeder tags the value mapping `construct=job`.
        implicit_covered: set = set()
        for parent, node in hub.get("node", {}).items():
            if parent not in plat_constructs:
                continue
            for fname, ftype in node.get("fields", {}).items():
                if not ftype.startswith("ref:"):
                    continue
                child = ftype[4:]
                if child not in plat_constructs:
                    continue
                if (parent, fname) in native_ref_fields:
                    continue
                # The bijective wrapper owns the pipeline->job->step
                # nesting (it creates the hub:job -has_step-> hub:step
                # edge directly). An implicit job->step containment
                # rule would create that same edge from a second
                # anchor, so in reverse two rules write the steps list
                # — suppress it when the wrapper is active.
                if (
                    bijective_owned_cst_tags
                    and parent == "job"
                    and child == "step"
                ):
                    continue
                # Only emit if the platform's IR mapping
                # `[<parent>.field.<fname>].<plat>` actually lists
                # a CST key — otherwise this is a hub-level edge
                # the platform doesn't have a native field for.
                platform_keys = (
                    ir.get(parent, {}).get("field", {}).get(fname, {}).get(plat, [])
                )
                # Exception: platforms whose jobs are TOP-LEVEL KEYLESS
                # mappings (gitlab — `build:` IS the job's name, tagged
                # `construct=job` by exclusion in the seeder, with no
                # fixed `jobs:` carrier key) never list a CST key in
                # ir.toml, so `platform_keys` is empty — yet the implicit
                # rule is their ONLY reverse-containment path (it binds
                # the entry key ↔ the job's name satellite). HEAD emitted
                # this rule; the carrier-elimination's `platform_keys`
                # guard wrongly conflated "no fixed key" with "no such
                # field" and dropped it, so gitlab reverse built no root
                # mapping ("no root mapping" in roundtrip_semantic).
                #
                # This is NOT derivable from the catalog (gitlab and
                # woodpecker are structurally identical here — both have
                # `job` in plat_constructs and no native `ref:job` field;
                # the difference is purely in the seeder's by-exclusion
                # tagging). It is declared per-platform via
                # `top_level_keyless_jobs` in targets.toml. Restoring it
                # broadly is regressive — e.g. woodpecker models work as
                # native `pipeline.steps`, so a second implicit job path
                # empties its IR (chaos woodpecker<->drone triangle).
                keyless_jobs = (
                    child == "job"
                    and not bijective_owned_cst_tags
                    and targets_cfg.get(plat, {}).get("top_level_keyless_jobs", False)
                )
                if not platform_keys and not keyless_jobs:
                    continue
                # Map-shape implicit only. The list-shape variant
                # (drone-style `steps: [...]`) was tried in v22 —
                # it shifted matrix score from 112 → 100 (−12 ✓
                # net) because reverse-cascade competition between
                # map and list shapes for the same hub:has_<child>
                # edge created divergent CST per pair. Re-enable
                # only when shape is determined per (parent, field,
                # platform) from the catalog type so exactly one
                # rule fires.
                for parent_cst in cst_aliases.get(parent, [parent]):
                    for child_cst in cst_aliases.get(child, [child]):
                        rules.append(implicit_containment_rule(
                            plat, parent, fname, child, parent_cst, child_cst,
                        ))
                implicit_covered.add((parent, fname))

        # Native fields the platform's manifest covers — used to
        # decide where carrier-comment fallbacks are needed.
        native_fields = {
            (r["to"].split(".", 1)[0], r["to"].split(".", 1)[1])
            for r in manifest.get("rule", [])
        }

        # IR-extending concept rules claim ownership of a target
        # ref field on a parent construct (e.g. `step_image` →
        # hub:step.image, `pipeline_default_image` →
        # hub:pipeline.image). Carrier-comment rules for those
        # same (parent, field) ref edges would double-emit hub:image
        # nodes on reverse — collected here so the carrier loop
        # below can skip them.
        concept_owned_refs: set = set()
        for concept_name, concept in concepts.items():
            if plat not in concept.get("platform_path", {}):
                continue
            tc = concept.get("target_construct")
            if not tc:
                continue
            edge = concept.get("edge", f"has_{tc}")
            # Translate edge "has_<X>" → field name "<X>" (ref name
            # in the hub schema, which is what the carrier loop
            # iterates over).
            field_name = edge[4:] if edge.startswith("has_") else edge
            concept_owned_refs.add((concept["parent"], field_name))

        # Carriers ARE the root evil for cross-platform IR
        # cleanliness (they mask missing TGG work — every gap
        # passes the roundtrip via annotation rather than via a
        # real rule, and coverage_analysis shows them at 0% fired
        # forward because chaos never synthesises `# @hub:`
        # syntax). Goal: eliminate carriers entirely.
        #
        # Tried full elimination once (May 27) — chaos roundtrip
        # immediately broke (travis lost its `jobs:` structure
        # because carriers had been the only emit path). Lesson:
        # remove carriers PER (parent, field) only after a real
        # native field rule or implicit_containment_rule covers
        # the gap. Until then, keep emitting them and treat each
        # carrier as a TODO marker for next-session field work.
        for parent, node in hub.get("node", {}).items():
            if parent not in plat_constructs:
                continue
            for fname, ftype in node.get("fields", {}).items():
                if (parent, fname) in native_fields:
                    continue
                if (parent, fname) in concept_owned_refs:
                    continue
                if (parent, fname) in implicit_covered:
                    continue
                if (parent, fname) in name_covered_by_parent_key:
                    # parent_key_name_rule handles map-keyed
                    # identity directly; suppressing the
                    # attr_carrier prevents the reverse cascade
                    # from also writing a redundant
                    # cst:CarrierComment[target_field=name].
                    continue
                # All remaining carriers DISABLED (May 27 phase 3).
                # ref_attr_carrier (depth-2 ref-field leaves) and
                # attr_carrier (depth-1 scalar fields) both never
                # fired forward in chaos because the walker never
                # emits `# @hub:` syntax. After replacing the only
                # actively-firing carrier family — synthesised name
                # carriers — with parent_key_name_rule, no chaos
                # test relies on them. Sandra's diagnosis was right:
                # they were a forever-fallback masking proper rule
                # work, not load-bearing infrastructure. Re-emit
                # per (parent, field) ONLY when a real fixture
                # writes `# @hub:` annotations the cascade needs to
                # round-trip; until then the absence is honest.
                _ = (parent, fname, ftype)  # suppress F841

        # Concept-rule key suppression. When a concept declares a
        # path like `on.push.branches` for this platform, the
        # outer key (`on`) is now "owned" by the concept's
        # hub-subgraph creation. Emitting a native field rule for
        # the same outer key alongside it creates two competing
        # MappingEntry chains in reverse (one from the concept
        # rule, one from the field rule), the entries collide on
        # GhostId but their value-subgraphs diverge, and emit
        # produces malformed nested-`on:\n  on:` output. Suppress
        # the native rule when a concept claims the same key.
        #
        # Tracked per (construct, key) pair — a concept that
        # claims `image` on step doesn't suppress
        # pipeline-level `image:` rules.
        concept_claimed: set = set()
        for concept_name, concept in concepts.items():
            pp = concept.get("platform_path", {})
            if plat in pp:
                outer_key = pp[plat].split(".")[0].rstrip("[]")
                # Suppression keys on the CST-side construct name —
                # field rules are emitted per (CST construct, field),
                # not per hub-construct.
                cst_parent_local = concept.get("platform_parent", {}).get(
                    plat, concept["parent"]
                )
                concept_claimed.add((cst_parent_local, outer_key))

        for r in manifest.get("rule", []):
            construct, _, field = r["to"].partition(".")
            if (construct, r.get("from")) in concept_claimed:
                continue  # owned by a concept rule
            fk = field_kind.get((construct, field), "scalar")
            ref_kind = fk[4:] if fk.startswith("ref:") else None
            shape = r["shape"]
            # The bijective rule (job-wrapper layer), rc7:
            # REPLACE the flat `pipeline.has_step` field rule with
            # the nested wrapper. The wrapper creates hub:job and
            # references hub:pipeline + hub:step as `role:
            # "References"` context corrs WITH span anchors — the
            # rc7 unblock that the empty-binding rc6 shortcut
            # couldn't express. `continue` skips the flat rule so
            # drone IR is nested-only (no double `pipeline.has_step`
            # + `pipeline.has_job.has_step`).
            if (
                shape == "seq_mapping_nodes"
                and ref_kind
                and construct == "pipeline"
                and ref_kind in co_created_cst_tags
            ):
                rules.append(nested_steps_to_jobs_rule(plat, r["from"], ref_kind))
                # Name the synthesised hub:job from the step item's
                # `name:` key so named-job targets (github, …) get a
                # well-defined `jobs:` map key. The key is whatever
                # the platform maps to job.name / step.name.
                name_key = next(
                    (
                        rr["from"]
                        for rr in manifest.get("rule", [])
                        if rr["to"] in ("job.name", "step.name")
                        and rr.get("shape") == "scalar_attr"
                    ),
                    None,
                )
                if name_key:
                    rules.append(
                        job_name_from_nested_step_rule(
                            plat, r["from"], ref_kind, name_key
                        )
                    )
                continue
            # A `map<string>` field the IR models as a ref construct (gitlab
            # variables→variable, inputs→parameter) carries SCALAR values, not
            # construct objects — the map_nodes REF form never matches
            # `{KEY: value}` and silently drops the content. Force the non-ref
            # scalar form (key→value attrs).
            if ref_kind and shape == "map_nodes" and is_scalar_map(r.get("type", "")):
                ref_kind = None
            # A NON-ref `list<object>` field (step.env, job.options, step.gate,
            # hook.phase, …) lowered to seq_attr expects SCALAR items and drops
            # the object content. Route to seq_block_attr (captures each object
            # item's key→value entries on a concrete hub:item).
            if not ref_kind and shape == "seq_attr" and is_object_list(r.get("type", "")):
                shape = "seq_block_attr"
            # Mixed `list<construct | scalar>` (gitlab services, buildkite
            # steps): use the concrete-item wrapper so the scalar arm and the
            # mapping arm don't compete for `coll -has_item-> rk`. The default
            # seq_mapping_nodes emits only the mapping arm → scalar items are
            # silently dropped; the per-element `hub:item{vkind}` carries the
            # variant and discriminates the two backward.
            if (ref_kind and shape == "seq_mapping_nodes"
                    and is_mixed_seq_union(r.get("type", ""))):
                # ref_kind has its own construct rule iff it maps to this
                # platform (construct_rule is emitted per plat_constructs); then
                # the item-map arm References it, else it Establishes the element.
                _has_rule = ref_kind in plat_constructs
                for cst in cst_aliases.get(construct, [construct]):
                    rules += seq_item_union_rules(
                        plat, construct, field, r["from"], ref_kind,
                        cst_construct=(cst if cst != construct else None),
                        child_has_construct_rule=_has_rule,
                    )
                continue
            # Emit one field rule per CST alias of this IR
            # construct. Without this, drone's manifest `to=job.image`
            # produces R_drone_job_image_image with L=construct=job
            # — which never matches because the seeder uses
            # construct=step / construct=step_docker / etc. for
            # drone. The alias loop anchors a field rule on every
            # CST tag the seeder might emit.
            cst_names = cst_aliases.get(construct, [construct])
            for cst in cst_names:
                # `job` field rules on a bijective-owned step tag
                # populate the suppressed hub:job-from-step path.
                # hub:job has no native fields for these flat-step
                # platforms (the wrapper synthesises it); the content
                # (commands, name) lands on hub:step via its own field
                # rules. Skipping these avoids a second reverse anchor.
                if construct == "job" and bijective_owned_cst_tags:
                    continue
                # On wrapper platforms the step name is carried by
                # hub:job (job_name_from_nested_step_rule), so the
                # generic step.name field rule must NOT also emit it —
                # otherwise reverse writes the `name:` entry twice
                # (once per anchor) under the one step mapping.
                if (
                    construct == "step"
                    and field == "name"
                    and cst in bijective_owned_cst_tags
                ):
                    continue
                if shape == "union":
                    subs = expand_union(r["type"], ref_kind is not None)
                    # `X | list<X>` expands to BOTH mapping_node (single
                    # source `field: {obj}`) and seq_mapping_nodes (list
                    # source `field: [{obj}]`). Forward both are needed —
                    # they match different source syntax. BACKWARD they
                    # compete for the same hub:attr→collection and the
                    # single shape wins, collapsing an N-item collection to
                    # one mapping (proven: gitlab cache 3→1). Make the
                    # single mapping_node FORWARD-ONLY in that case (IM↔rk
                    # References, like seq_mapping_nodes already is) so only
                    # seq_mapping_nodes reconstructs backward — as a list,
                    # which re-seeds to the identical hub collection (hub-
                    # lossless; single-vs-list is pure surface syntax).
                    both_construct_shapes = (
                        "mapping_node" in subs and "seq_mapping_nodes" in subs
                    )
                    for sub in subs:
                        is_single_union_arm = (
                            both_construct_shapes and sub == "mapping_node"
                        )
                        # Canonical-list platforms: DROP the single mapping_node
                        # shape. The seeder canonicalises a single mapping into
                        # a one-item sequence (CST sugar → list), so
                        # seq_mapping_nodes is the SOLE, bijective rule both
                        # directions — no ambiguous backward competitor that
                        # collapses N-item collections (Sandra's 1:3 = modeling
                        # weakness, fixed by the canonical-list model, NOT a
                        # direction flag).
                        if is_single_union_arm and plat in CANONICAL_LIST_PLATFORMS:
                            continue
                        rules += _named(
                            field_rule(
                                plat, construct, field, r["from"], sub, ref_kind, cst,
                                single_forward_only=is_single_union_arm,
                            ),
                            sub,
                        )
                else:
                    rules += field_rule(plat, construct, field, r["from"], shape, ref_kind, cst)

        # Semantic concept rules: declared in concepts.toml as
        # path-to-hub-subgraph mappings. One rule per (platform,
        # concept) entry — multiple platforms converge on the
        # same hub_path so chaos cross-triangle pairs them up.
        for concept_name, concept in concepts.items():
            pp = concept.get("platform_path", {})
            if plat not in pp:
                continue
            r = concept_rule(plat, concept_name, concept, pp[plat])
            if r is not None:
                rules.append(r)

        # Bespoke per-platform structural bridges (not derivable from the
        # manifest shapes): bitbucket's job containment runs through the
        # self-referential `pipelines:` map + a `- step:` wrapper hop;
        # parallel groups and the five event selectors add one structural
        # level each (stage 2).
        if plat == "bitbucket":
            rules.append(bitbucket_default_steps_rule())
            rules.append(bitbucket_parallel_group_rule())
            rules.append(bitbucket_parallel_steps_rule())
            rules.append(bitbucket_parallel_expanded_group_rule())
            rules.append(bitbucket_parallel_expanded_fail_fast_rule("true"))
            rules.append(bitbucket_parallel_expanded_fail_fast_rule("false"))
            rules.append(bitbucket_parallel_expanded_steps_rule())
            for selector, attr_name, group_vkind in BITBUCKET_SELECTORS:
                rules.append(bitbucket_selector_group_rule(selector, group_vkind))
                rules.append(bitbucket_selector_steps_rule(selector, attr_name, group_vkind))
                rules.append(bitbucket_selector_parallel_steps_rule(selector, attr_name, group_vkind))
                rules.append(bitbucket_selector_parallel_expanded_steps_rule(selector, attr_name, group_vkind))

        ruleset = {
            "name": f"{plat}",
            "description": f"Generated TGG ruleset: {plat} CST <-> Hub-IR. Bidirectional — the engine derives CST->IR and IR->CST from this one set (model a).",
            "rules": rules,
        }
        (out_dir / f"{plat}.ruleset.json").write_text(json.dumps(ruleset, indent=1))
        total += len(rules)
        summary.append((plat, len(rules)))

    for plat, c in summary:
        print(f"  {plat:18s} {c:4d} rules")
    print(f"  TOTAL {total} rules across {len(summary)} platforms")


def _named(rules, suffix):
    """Disambiguate the per-arm rules of an expanded union."""
    for r in rules:
        r["name"] = f"{r['name']}_{suffix}"
    return rules


def _load(name):
    with open(CAT / name, "rb") as f:
        return tomllib.load(f)


if __name__ == "__main__":
    main()
