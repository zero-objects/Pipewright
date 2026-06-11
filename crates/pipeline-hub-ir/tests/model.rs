//! Tests for `model.rs` — constructors, defaults, and serde round-trip
//! for each top-level Hub-IR type.

use indexmap::IndexMap;
use pipeline_hub_ir::{
    AllowFailure, ArtifactRef, ArtifactSpec, BranchProtection, CacheSpec, ConcurrencySpec,
    CorrespondenceEvent, DagSource, Duration, Edge, EdgeKind, Environment, Job, JobIdentity,
    JobType, Lockfile, MatrixSpec, ParameterSpec, PermissionLevel, PermissionsSpec, Pipeline,
    Provenance, RenameRecord, RetrySpec, ScopeFilter, ServiceSpec, SplitPart, Step, Trigger,
    TriggerKind, TriggerStrategy, VariableScope, VariableSpec,
};
use pretty_assertions::assert_eq;

fn dummy_prov() -> Provenance {
    Provenance::from_byte_span("file.yml", "x", (0, 1))
}

#[test]
fn pipeline_new_creates_empty_pipeline() {
    let p = Pipeline::new("pipeline-1", dummy_prov());
    assert_eq!(p.id, "pipeline-1");
    assert!(p.jobs.is_empty());
    assert!(p.edges.is_empty());
    assert!(p.workflow_rules.is_empty());
    assert!(p.permissions.is_none());
    assert!(p.concurrency.is_none());
    assert!(p.lockfile_ref.is_none());
}

#[test]
fn job_new_creates_standard_job() {
    let id = JobIdentity::fresh("u-1");
    let j = Job::new("j-1", id.clone(), "build", dummy_prov());
    assert_eq!(j.id, "j-1");
    assert_eq!(j.name, "build");
    assert_eq!(j.identity, id);
    assert_eq!(j.r#type, JobType::Standard);
    assert!(j.steps.is_empty());
    assert!(j.correspondence_trail.is_empty());
    assert!(j.conditions.is_empty());
}

#[test]
fn job_type_defaults_to_standard() {
    assert_eq!(JobType::default(), JobType::Standard);
}

#[test]
fn correspondence_event_variants_round_trip() {
    let events = vec![
        CorrespondenceEvent::RenameFrom {
            previous_name: "old".into(),
        },
        CorrespondenceEvent::InsertedFor {
            source_job_id: "j-1".into(),
            at_step_index: 2,
        },
        CorrespondenceEvent::SplitOf {
            source_job_id: "j-1".into(),
            part: SplitPart::First,
            split_after: 3,
        },
        CorrespondenceEvent::MergedInto {
            target_job_id: "j-2".into(),
        },
        CorrespondenceEvent::TggMaterialized {
            rule_id: "R_X".into(),
            at: "2026-05-14T12:00:00Z".into(),
        },
    ];
    let json = serde_json::to_string(&events).expect("ser");
    let round: Vec<CorrespondenceEvent> = serde_json::from_str(&json).expect("de");
    assert_eq!(events, round);
}

#[test]
fn trigger_kind_serializes_as_expected_variant_names() {
    let kinds = [
        TriggerKind::Push,
        TriggerKind::MergeRequest,
        TriggerKind::Tag,
        TriggerKind::Schedule,
        TriggerKind::Manual,
        TriggerKind::External,
        TriggerKind::RepositoryDispatch,
        TriggerKind::Webhook,
        TriggerKind::PollScm,
    ];
    for k in kinds {
        let json = serde_json::to_string(&k).expect("ser");
        let round: TriggerKind = serde_json::from_str(&json).expect("de");
        assert_eq!(k, round);
    }
}

#[test]
fn trigger_round_trip() {
    let t = Trigger {
        id: "t-1".into(),
        kind: TriggerKind::Schedule,
        provenance: dummy_prov(),
        branch_filter: vec!["main".into()],
        path_filter: vec![],
        tag_filter: vec![],
        schedule_expr: Some("0 0 * * *".into()),
        event_payload_filter: IndexMap::new(),
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&t).expect("ser");
    let round: Trigger = serde_json::from_str(&json).expect("de");
    assert_eq!(t, round);
}

#[test]
fn environment_round_trip() {
    let e = Environment {
        id: "env-1".into(),
        name: "production".into(),
        provenance: dummy_prov(),
        approval_required: true,
        approval_reviewers: vec!["alice".into()],
        protection_rules: vec![],
        url_template: Some("https://prod.example.com/$JOB".into()),
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&e).expect("ser");
    let round: Environment = serde_json::from_str(&json).expect("de");
    assert_eq!(e, round);
}

#[test]
fn permissions_spec_with_scopes_round_trip() {
    let mut scopes = IndexMap::new();
    scopes.insert("contents".into(), PermissionLevel::Read);
    scopes.insert("packages".into(), PermissionLevel::Write);
    let p = PermissionsSpec {
        scopes,
        branches: vec![BranchProtection {
            pattern: "main".into(),
            require_approval: true,
            required_roles: vec!["maintainer".into()],
        }],
        rbac_role: None,
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&p).expect("ser");
    let round: PermissionsSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(p, round);
}

#[test]
fn concurrency_spec_round_trip() {
    let c = ConcurrencySpec {
        group: "deploy".into(),
        cancel_in_progress: true,
        resource: Some("lock-A".into()),
    };
    let json = serde_json::to_string(&c).expect("ser");
    let round: ConcurrencySpec = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn variable_scope_with_filter_round_trip() {
    let mut vars = IndexMap::new();
    vars.insert("CI_DEBUG".into(), VariableSpec::literal("true"));
    let scope = VariableScope {
        id: "scope-1".into(),
        variables: vars,
        applies_to: ScopeFilter::InStages(vec!["build".into(), "test".into()]),
    };
    let json = serde_json::to_string(&scope).expect("ser");
    let round: VariableScope = serde_json::from_str(&json).expect("de");
    assert_eq!(scope, round);
}

#[test]
fn scope_filter_all_variants_round_trip() {
    let filters = [
        ScopeFilter::AllJobs,
        ScopeFilter::InStages(vec!["s".into()]),
        ScopeFilter::InJobs(vec!["j".into()]),
        ScopeFilter::MatchingTags(vec!["t".into()]),
    ];
    for f in &filters {
        let json = serde_json::to_string(f).expect("ser");
        let round: ScopeFilter = serde_json::from_str(&json).expect("de");
        assert_eq!(f, &round);
    }
}

#[test]
fn lockfile_empty_uses_default_threshold() {
    let lf = Lockfile::empty();
    assert!((lf.content_recovery_threshold - 0.7).abs() < f32::EPSILON);
    assert_eq!(lf.schema_version, "0.1.0");
    assert!(lf.paths.is_empty());
    assert!(lf.renames.is_empty());
}

#[test]
fn lockfile_round_trip_with_renames() {
    let mut paths = IndexMap::new();
    paths.insert("jobs.build".into(), "00000001-7e3a9f2b4c8d1e6f".into());
    let lf = Lockfile {
        schema_version: "0.1.0".into(),
        paths,
        renames: vec![RenameRecord {
            from: "jobs.old".into(),
            to: "jobs.build".into(),
            at: "2026-05-14T10:00:00Z".into(),
        }],
        content_recovery_threshold: 0.7,
        content_signatures: IndexMap::new(),
    };
    let json = serde_json::to_string(&lf).expect("ser");
    let round: Lockfile = serde_json::from_str(&json).expect("de");
    assert_eq!(lf, round);
}

#[test]
fn allow_failure_flag_round_trip() {
    let cases = [AllowFailure::Flag(true), AllowFailure::Flag(false)];
    for f in cases {
        let json = serde_json::to_string(&f).expect("ser");
        let round: AllowFailure = serde_json::from_str(&json).expect("de");
        assert_eq!(f, round);
    }
}

#[test]
fn allow_failure_exit_codes_round_trip() {
    let f = AllowFailure::ExitCodes {
        exit_codes: vec![1, 42],
    };
    let json = serde_json::to_string(&f).expect("ser");
    let round: AllowFailure = serde_json::from_str(&json).expect("de");
    assert_eq!(f, round);
}

#[test]
fn edge_kind_round_trip() {
    let kinds = [
        EdgeKind::DependsOnHard,
        EdgeKind::DependsOnSoft,
        EdgeKind::StageImplicit,
        EdgeKind::Triggers(TriggerStrategy::Depend),
        EdgeKind::Triggers(TriggerStrategy::FireAndForget),
        EdgeKind::Triggers(TriggerStrategy::MirrorStatus),
    ];
    for k in &kinds {
        let json = serde_json::to_string(k).expect("ser");
        let round: EdgeKind = serde_json::from_str(&json).expect("de");
        assert_eq!(k, &round);
    }
}

#[test]
fn edge_round_trip() {
    let e = Edge {
        from: "j-1".into(),
        to: "j-2".into(),
        kind: EdgeKind::DependsOnHard,
        provenance: dummy_prov(),
    };
    let json = serde_json::to_string(&e).expect("ser");
    let round: Edge = serde_json::from_str(&json).expect("de");
    assert_eq!(e, round);
}

#[test]
fn dag_source_variants_round_trip() {
    for d in [
        DagSource::ExplicitNeeds,
        DagSource::StageImplicit,
        DagSource::Mixed,
    ] {
        let json = serde_json::to_string(&d).expect("ser");
        let round: DagSource = serde_json::from_str(&json).expect("de");
        assert_eq!(d, round);
    }
}

#[test]
fn matrix_spec_round_trip() {
    let mut axes = IndexMap::new();
    axes.insert("os".into(), vec!["linux".into(), "macos".into()]);
    axes.insert("toolchain".into(), vec!["stable".into(), "nightly".into()]);
    let m = MatrixSpec {
        axes,
        include: vec![],
        exclude: vec![],
    };
    let json = serde_json::to_string(&m).expect("ser");
    let round: MatrixSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(m, round);
}

#[test]
fn retry_spec_round_trip() {
    let r = RetrySpec {
        max: 3,
        when: vec!["runner_system_failure".into()],
        exit_codes: vec![137],
    };
    let json = serde_json::to_string(&r).expect("ser");
    let round: RetrySpec = serde_json::from_str(&json).expect("de");
    assert_eq!(r, round);
}

#[test]
fn cache_spec_round_trip() {
    let c = CacheSpec {
        key: Some("v1-cargo-$CI_JOB_NAME".into()),
        paths: vec!["target/".into(), "~/.cargo/registry".into()],
        policy: Some("pull-push".into()),
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&c).expect("ser");
    let round: CacheSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(c, round);
}

#[test]
fn artifact_spec_round_trip() {
    let a = ArtifactSpec {
        name: "build-output".into(),
        paths: vec!["target/release/*".into()],
        when: Some("on_success".into()),
        expire_in: Some(Duration::new("1 week")),
        sub_type: vec!["dotenv".into()],
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&a).expect("ser");
    let round: ArtifactSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(a, round);
}

#[test]
fn artifact_ref_round_trip() {
    let r = ArtifactRef {
        name: "build-output".into(),
        from_job: Some("build".into()),
    };
    let json = serde_json::to_string(&r).expect("ser");
    let round: ArtifactRef = serde_json::from_str(&json).expect("de");
    assert_eq!(r, round);
}

#[test]
fn service_spec_round_trip() {
    let s = ServiceSpec {
        name: "postgres:15".into(),
        alias: Some("db".into()),
        command: vec!["--port=5432".into()],
        opaque: IndexMap::new(),
    };
    let json = serde_json::to_string(&s).expect("ser");
    let round: ServiceSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(s, round);
}

#[test]
fn parameter_spec_round_trip() {
    let p = ParameterSpec {
        name: "VERSION".into(),
        r#type: Some("string".into()),
        default: Some("1.0".into()),
        description: Some("Release version".into()),
    };
    let json = serde_json::to_string(&p).expect("ser");
    let round: ParameterSpec = serde_json::from_str(&json).expect("de");
    assert_eq!(p, round);
}

#[test]
fn full_pipeline_round_trip_with_job_and_edge() {
    let mut p = Pipeline::new("pipeline-1", dummy_prov());
    let j = Job::new(
        "j-build",
        JobIdentity::fresh("u-build"),
        "build",
        dummy_prov(),
    );
    let j2 = Job::new("j-test", JobIdentity::fresh("u-test"), "test", dummy_prov());
    p.jobs.push(j);
    p.jobs.push(j2);
    p.edges.push(Edge {
        from: "j-build".into(),
        to: "j-test".into(),
        kind: EdgeKind::DependsOnHard,
        provenance: dummy_prov(),
    });
    let json = serde_json::to_string(&p).expect("ser");
    let round: Pipeline = serde_json::from_str(&json).expect("de");
    assert_eq!(p, round);
}

#[test]
fn job_with_step_round_trip() {
    let mut j = Job::new("j-1", JobIdentity::fresh("u-1"), "build", dummy_prov());
    j.steps.push(Step {
        command: "cargo build --release".into(),
        kind: pipeline_hub_ir::StepKind::Run,
        provenance: dummy_prov(),
        style: pipeline_hub_ir::ScalarStyle::Plain,
        leading_comments: Vec::new(),
        opaque: indexmap::IndexMap::new(),
    });
    let json = serde_json::to_string(&j).expect("ser");
    let round: Job = serde_json::from_str(&json).expect("de");
    assert_eq!(j, round);
}

#[test]
fn duration_preserves_source_string() {
    let d = Duration::new("5 min");
    assert_eq!(d.0, "5 min");
    let json = serde_json::to_string(&d).expect("ser");
    let round: Duration = serde_json::from_str(&json).expect("de");
    assert_eq!(d, round);
}

#[test]
fn variable_spec_literal_constructor() {
    let v = VariableSpec::literal("hello");
    assert_eq!(v.value, "hello");
    assert!(v.description.is_none());
    assert!(!v.expanded);
}
