mod common;

use common::taste_fixture;
use std::fs;
use std::path::Path;

const TASTE_SESSION_ID: &str = "00000000-0000-4000-8000-000000000031";

#[test]
fn taste_fixture_corpus_has_required_session_shapes() {
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let content = fs::read_to_string(&session).expect("read taste session fixture");

    for required in [
        "\"type\":\"file-history-snapshot\"",
        "\"trackedFileBackups\":[{\"filePath\"",
        "\"content\":",
        "\"backupFileName\":\"fixture-a1b2c3d4@v1\"",
        "\"backupFileName\":\"fixture-writenew1@v1\"",
        "\"backupFileName\":\"fixture-subagent1@v1\"",
        "\"name\":\"Edit\"",
        "\"name\":\"Write\"",
        "\"name\":\"Bash\"",
        "\"name\":\"NotebookEdit\"",
        "\"name\":\"mcp_fixture_codedb_edit\"",
        "mcp_marker",
        "new_string was NOT written",
        "toolu_taste_edit_revert",
        "toolu_taste_notebook_edit",
        "toolu_taste_edit_partial",
        "partial_drop",
        "human_edited_marker",
        "toolu_taste_edit_manual",
        "toolu_taste_edit_missing_final",
        "missing_final_marker",
        "toolu_taste_edit_untracked",
        "untracked_marker",
        "notebook_marker",
    ] {
        assert!(
            content.contains(required),
            "taste session fixture should contain {required}"
        );
    }
}

#[test]
fn taste_fixture_corpus_has_snapshot_sidecar_blobs() {
    for version in ["v1", "v2", "v3"] {
        let sidecar = taste_fixture(&format!(
            "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-a1b2c3d4@{version}"
        ));
        assert!(
            sidecar.exists(),
            "missing sidecar fixture-a1b2c3d4@{version}"
        );
        let body = fs::read_to_string(sidecar).expect("read sidecar");
        assert!(
            body.contains("taste fixture baseline"),
            "sidecar @{version} should contain baseline marker"
        );
    }
}

#[test]
fn taste_fixture_corpus_has_write_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-writenew1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-writenew1@v1");
    let body = fs::read_to_string(sidecar).expect("read write sidecar");
    assert!(
        body.contains("written by Write tool fixture"),
        "write sidecar should contain Write fixture marker"
    );
}

#[test]
fn taste_fixture_corpus_has_subagent_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-subagent1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-subagent1@v1");
    let body = fs::read_to_string(sidecar).expect("read subagent sidecar");
    assert!(
        body.contains("subagent_marker"),
        "subagent sidecar should contain subagent edit marker"
    );
}

#[test]
fn taste_fixture_corpus_has_manual_edit_sidecar_blob() {
    let sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-manual1@v1"
    ));
    assert!(sidecar.exists(), "missing sidecar fixture-manual1@v1");
    let body = fs::read_to_string(sidecar).expect("read manual sidecar");
    assert!(
        body.contains("human_edited_marker"),
        "manual sidecar should contain human IDE edit marker"
    );
}

#[test]
fn taste_fixture_corpus_has_missing_final_sidecar_blob_only_for_intermediate_snapshot() {
    let intermediate = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-missingfinal1@v1"
    ));
    assert!(
        intermediate.exists(),
        "missing sidecar fixture-missingfinal1@v1"
    );
    let body = fs::read_to_string(intermediate).expect("read missing-final sidecar");
    assert!(
        body.contains("missing_final_marker"),
        "intermediate sidecar should contain missing-final marker"
    );

    let final_sidecar = taste_fixture(&format!(
        "claude-cli/file-history/{TASTE_SESSION_ID}/fixture-missingfinal1@v2"
    ));
    assert!(
        !final_sidecar.exists(),
        "final sidecar fixture-missingfinal1@v2 should be absent to simulate R1 degradation"
    );
}

#[test]
fn taste_fixture_corpus_has_subagent_edit_sidechain() {
    let subagent = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}/subagents/agent-taste000000000001.jsonl"
    ));
    let content = fs::read_to_string(subagent).expect("read taste subagent fixture");

    for required in [
        "\"isSidechain\":true",
        "\"name\":\"Edit\"",
        "taste_subagent.rs",
        "subagent_marker",
    ] {
        assert!(
            content.contains(required),
            "taste subagent fixture should contain {required}"
        );
    }
}

#[test]
fn taste_fixture_readme_documents_coverage() {
    let readme = fs::read_to_string(taste_fixture("README.md")).expect("read taste README");

    for required in [
        TASTE_SESSION_ID,
        "backupFileName",
        "permission denial",
        "subagent",
        "file-history",
    ] {
        assert!(
            readme.contains(required),
            "taste README should document {required}"
        );
    }
}

#[test]
fn committed_taste_fixtures_do_not_contain_known_local_sensitive_markers() {
    let markers = [
        "/Users/season",
        "season179",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "ghp_",
        "sk-",
    ];

    for entry in fs::read_dir(taste_fixture("")).expect("read taste fixture root") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            check_taste_tree(&path, &markers);
        } else {
            check_taste_file(&path, &markers);
        }
    }
}

fn check_taste_tree(path: &std::path::Path, markers: &[&str]) {
    for entry in fs::read_dir(path).expect("read fixture directory") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            check_taste_tree(&path, markers);
        } else {
            check_taste_file(&path, markers);
        }
    }
}

fn check_taste_file(path: &std::path::Path, markers: &[&str]) {
    let content = fs::read_to_string(path).expect("fixture should be utf8");
    for marker in markers {
        assert!(
            !content.contains(marker),
            "{} contains sensitive marker {marker}",
            path.display()
        );
    }
}

#[test]
fn taste_extraction_plan_documents_implemented_status_and_r3_exclusion() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "Status: **IMPLEMENTED**",
        "Decision: no",
        "out of scope for taste-extraction",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document {required}"
        );
    }
}

#[test]
fn command_code_taste_formula_links_to_jottrace_implementation() {
    let formula = fs::read_to_string("notes/command-code-taste-formula.md")
        .expect("read command code taste formula notes");

    for required in [
        "## Jottrace implementation",
        "jottrace taste",
        "notes/taste-extraction-plan.md",
        "IMPLEMENTED",
    ] {
        assert!(
            formula.contains(required),
            "command code taste formula should link to jottrace implementation via {required}"
        );
    }
}

#[test]
fn taste_extraction_plan_implementation_complete() {
    for path in [
        // Step 1 — fixture corpus
        "tests/fixtures/taste/README.md",
        // Step 2 — shared Claude parse layer
        "src/taste/parse.rs",
        "tests/taste_parse.rs",
        // Step 3 — snapshot sidecar resolver
        "src/taste/sidecar.rs",
        "tests/taste_sidecar.rs",
        // Step 4 — file_timelines materialization
        "src/migrations/010_taste_extraction.sql",
        "src/taste/timeline.rs",
        "tests/taste_timeline.rs",
        // Step 5 — preference compiler
        "src/taste/compiler.rs",
        "tests/taste_compiler.rs",
        // Step 6 — preference_examples + CLI
        "src/migrations/011_preference_examples.sql",
        "tests/taste_preference_examples.rs",
        "src/taste/extract.rs",
        "src/taste/show.rs",
        "src/taste/export.rs",
        "tests/taste_extract.rs",
        "tests/taste_show_timeline.rs",
        "tests/taste_show_example.rs",
        "tests/taste_export.rs",
        // Step 7 — coverage report
        "src/taste/status.rs",
        "tests/taste_status.rs",
        // Follow-on migrations
        "src/migrations/012_preference_examples_mcp_evidence.sql",
        "src/migrations/013_taste_extractions.sql",
    ] {
        assert!(Path::new(path).exists(), "plan artifact missing: {path}");
    }

    let migration_010 =
        fs::read_to_string("src/migrations/010_taste_extraction.sql").expect("read migration 010");
    assert!(
        migration_010.contains("file_timelines"),
        "migration 010 should define file_timelines"
    );

    let migration_011 = fs::read_to_string("src/migrations/011_preference_examples.sql")
        .expect("read migration 011");
    assert!(
        migration_011.contains("preference_examples"),
        "migration 011 should define preference_examples"
    );

    let migration_012 = fs::read_to_string("src/migrations/012_preference_examples_mcp_evidence.sql")
        .expect("read migration 012");
    assert!(
        migration_012.contains("mcp_correlation"),
        "migration 012 should allow mcp_correlation evidence_kind"
    );

    let migration_013 =
        fs::read_to_string("src/migrations/013_taste_extractions.sql").expect("read migration 013");
    assert!(
        migration_013.contains("taste_extractions"),
        "migration 013 should define taste_extractions"
    );

    let main_rs = fs::read_to_string("src/main.rs").expect("read main.rs");
    for required in [
        "jottrace taste extract",
        "jottrace taste status",
        "jottrace taste show timeline",
        "jottrace taste show example",
        "jottrace taste export",
    ] {
        assert!(
            main_rs.contains(required),
            "main.rs should document {required}"
        );
    }

    assert_eq!(jottrace::storage::LATEST_SCHEMA_VERSION, 13);
    assert_eq!(jottrace::taste::EXTRACTOR_VERSION, "0.1.11");
    assert_eq!(jottrace::taste::HIGH_CONFIDENCE_THRESHOLD, 1.0);
}

#[test]
fn taste_extraction_documentation_complete() {
    let design = fs::read_to_string("docs/design.md").expect("read design.md");
    for required in [
        "## Taste extraction",
        "jottrace taste",
        "file_timelines",
        "preference_examples",
        "taste_extractions",
        "notes/taste-extraction-plan.md",
        "show timeline",
        "show example",
        "export --format jsonl",
        "--force",
        "R3 exception",
    ] {
        assert!(
            design.contains(required),
            "docs/design.md should document taste extraction via {required}"
        );
    }

    let inventory =
        fs::read_to_string("docs/reader-source-inventory.md").expect("read reader inventory");
    for required in [
        "tasks/*.output",
        "excluded from taste-extraction scope",
        "notes/taste-extraction-plan.md",
    ] {
        assert!(
            inventory.contains(required),
            "docs/reader-source-inventory.md should document taste R3 exclusion via {required}"
        );
    }

    let processor =
        fs::read_to_string("docs/processor-design.md").expect("read processor-design.md");
    for required in [
        "Relationship to taste extraction",
        "jottrace taste",
        "src/taste/parse.rs",
        "notes/taste-extraction-plan.md",
    ] {
        assert!(
            processor.contains(required),
            "docs/processor-design.md should document taste extraction via {required}"
        );
    }

    let readme = fs::read_to_string("README.md").expect("read README.md");
    for required in [
        "## Taste Extraction",
        "jottrace taste extract",
        "jottrace taste status",
        "jottrace taste show timeline",
        "jottrace taste show example",
        "jottrace taste export",
        "--force",
    ] {
        assert!(
            readme.contains(required),
            "README.md should document taste extraction via {required}"
        );
    }

    let changelog = fs::read_to_string("CHANGELOG.md").expect("read CHANGELOG.md");
    for required in [
        "## v26.7.0",
        "jottrace taste",
        "file_timelines",
        "preference_examples",
        "taste_extractions",
        "tasks/*.output",
    ] {
        assert!(
            changelog.contains(required),
            "CHANGELOG.md should document taste extraction release via {required}"
        );
    }
}

#[test]
fn taste_extraction_risk_coverage_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "**R1 — Snapshot sidecars.**",
        "**R2 — Bash attribution is best-effort.**",
        "**R3 — Async Task transcripts are not ingested (excluded).**",
        "**R4 — Manual human edits and untracked paths.**",
        "notes/command-code-taste-formula.md",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document risk coverage via {required}"
        );
    }

    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let content = fs::read_to_string(&session).expect("read taste session fixture");

    for required in [
        "toolu_taste_edit_missing_final",
        "fixture-missingfinal1@v2",
        "toolu_taste_bash",
        "toolu_taste_mcp_edit",
        "toolu_taste_edit_manual",
        "toolu_taste_edit_untracked",
    ] {
        assert!(
            content.contains(required),
            "taste fixture should cover plan risks via {required}"
        );
    }

    let readme = fs::read_to_string(taste_fixture("README.md")).expect("read taste README");
    for required in ["(R1)", "(R4)"] {
        assert!(
            readme.contains(required),
            "taste README should document risk coverage via {required}"
        );
    }

    let sidecar = fs::read_to_string("src/taste/sidecar.rs").expect("read sidecar module");
    assert!(
        sidecar.contains("MissingSidecar"),
        "sidecar resolver should handle R1 missing sidecars"
    );

    let compiler = fs::read_to_string("src/taste/compiler.rs").expect("read compiler module");
    for required in ["BashCorrelation", "McpCorrelation", "MissingFinalState"] {
        assert!(
            compiler.contains(required),
            "preference compiler should implement risk evidence via {required}"
        );
    }
}

#[test]
fn taste_extraction_locked_decisions_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "**Source scope — Claude only.**",
        "**Tool scope — complete coverage, not incremental.**",
        "**Subagents — merged into the parent timeline.**",
        "**Accept definition — present-at-session-end.**",
        "**Privacy — not a concern for this user.**",
        "**Execution model — separate command, materialized.**",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document locked decision via {required}"
        );
    }

    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    for required in ["claude_cli", "list_parent_claude_sessions", "EXTRACTOR_VERSION"] {
        assert!(
            extract.contains(required),
            "extract module should implement Claude-only materialized extraction via {required}"
        );
    }

    let parse = fs::read_to_string("src/taste/parse.rs").expect("read parse module");
    assert!(
        parse.contains("merge_streams"),
        "parse layer should merge subagent streams into parent timeline"
    );

    let compiler = fs::read_to_string("src/taste/compiler.rs").expect("read compiler module");
    assert!(
        compiler.contains("classify_present_at_session_end"),
        "compiler should use present-at-session-end accept definition"
    );

    let export = fs::read_to_string("src/taste/export.rs").expect("read export module");
    for required in ["proposal_content", "context"] {
        assert!(
            export.contains(required),
            "export should emit full proposal/context content via {required}"
        );
    }

    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let content = fs::read_to_string(&session).expect("read taste session fixture");

    for required in [
        "\"name\":\"Edit\"",
        "\"name\":\"Write\"",
        "\"name\":\"Bash\"",
        "\"name\":\"NotebookEdit\"",
        "\"name\":\"mcp_fixture_codedb_edit\"",
        "taste_subagent.rs",
        "fixture-subagent1@v1",
    ] {
        assert!(
            content.contains(required),
            "taste fixture should cover complete tool scope via {required}"
        );
    }

    let migration_010 =
        fs::read_to_string("src/migrations/010_taste_extraction.sql").expect("read migration 010");
    let migration_011 = fs::read_to_string("src/migrations/011_preference_examples.sql")
        .expect("read migration 011");
    for required in ["file_timelines", "preference_examples", "extractor_version"] {
        assert!(
            migration_010.contains(required) || migration_011.contains(required),
            "migrations should materialize taste tables with {required}"
        );
    }
}

#[test]
fn taste_extraction_plan_corrections_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "**C1 — Snapshots are not always inline.**",
        "**C2 — `edited` is not a peer outcome.**",
        "**C3 — `tool_result` success ≠ accept.**",
        "**C4 — Bash/MCP attribution is structurally lossy.**",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document correction via {required}"
        );
    }

    let sidecar = fs::read_to_string("src/taste/sidecar.rs").expect("read sidecar module");
    for required in ["backupFileName", "MissingSidecar"] {
        assert!(
            sidecar.contains(required),
            "sidecar resolver should implement C1 via {required}"
        );
    }

    let compiler = fs::read_to_string("src/taste/compiler.rs").expect("read compiler module");
    for required in [
        "classify_present_at_session_end",
        "PreferenceOutcome::Edited",
        "BashCorrelation",
        "McpCorrelation",
    ] {
        assert!(
            compiler.contains(required),
            "preference compiler should implement corrections via {required}"
        );
    }

    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let content = fs::read_to_string(&session).expect("read taste session fixture");

    for required in [
        "backupFileName",
        "toolu_taste_edit_partial",
        "toolu_taste_edit_accept",
        "toolu_taste_edit_revert",
        "toolu_taste_bash",
        "toolu_taste_mcp_edit",
    ] {
        assert!(
            content.contains(required),
            "taste fixture should cover plan corrections via {required}"
        );
    }
}

#[test]
fn taste_extraction_architecture_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## Architecture",
        "Timeline-first",
        "file_timelines",
        "preference_examples",
        "[Claude event parser]",
        "[preference compiler:",
        "`jottrace taste show timeline`",
        "(context, chosen, rejected)",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document architecture via {required}"
        );
    }

    let migration_010 =
        fs::read_to_string("src/migrations/010_taste_extraction.sql").expect("read migration 010");
    for required in [
        "source_session_id",
        "file_path",
        "seq",
        "event_seq",
        "content",
        "trigger_event_ref",
        "source_kind",
        "inline_snapshot",
        "sidecar_snapshot",
        "missing_sidecar",
    ] {
        assert!(
            migration_010.contains(required),
            "file_timelines schema should match plan architecture via {required}"
        );
    }

    let migration_011 = fs::read_to_string("src/migrations/011_preference_examples.sql")
        .expect("read migration 011");
    for required in [
        "generation",
        "proposal_event_seq",
        "tool_use_id",
        "proposal_content",
        "context",
        "outcome",
        "confidence",
        "evidence_kind",
        "extractor_version",
        "'accepted'",
        "'rejected'",
        "'edited'",
    ] {
        assert!(
            migration_011.contains(required),
            "preference_examples schema should match plan architecture via {required}"
        );
    }

    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    for required in [
        "parse_jsonl",
        "merge_streams",
        "SnapshotSidecarResolver",
        "FileTimelineMaterializer::materialize",
        "PreferenceCompiler::compile",
        "replace_session_file_timelines",
        "replace_session_preference_examples",
    ] {
        assert!(
            extract.contains(required),
            "extract pipeline should wire architecture stages via {required}"
        );
    }

    let export = fs::read_to_string("src/taste/export.rs").expect("read export module");
    for required in ["context", "chosen", "rejected", "TasteExportRecord"] {
        assert!(
            export.contains(required),
            "export should emit trainer-facing triples via {required}"
        );
    }

    let show = fs::read_to_string("src/taste/show.rs").expect("read show module");
    assert!(
        show.contains("run_taste_show_timeline"),
        "show module should expose inspectable file_timelines via run_taste_show_timeline"
    );
}

#[test]
fn taste_extraction_cli_surface_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## CLI surface",
        "`jottrace taste extract [--session <id>]`",
        "`jottrace taste status`",
        "`jottrace taste show timeline --session <id> --file <path>`",
        "`jottrace taste show example <id>`",
        "`jottrace taste export --format jsonl [--out <path>]`",
        "(context, chosen, rejected)",
        "Idempotent",
        "extractor_version",
        "coverage (% of file-modifying events resolved)",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document CLI surface via {required}"
        );
    }

    let main_rs = fs::read_to_string("src/main.rs").expect("read main.rs");
    for required in [
        "run_taste_extract_command",
        "run_taste_status_command",
        "run_taste_show_timeline_command",
        "run_taste_show_example_command",
        "run_taste_export_command",
        "parse_taste_extract_command",
        "parse_taste_export_command",
        "--session",
        "--force",
        "--details",
        "--file",
        "--format",
        "--out",
    ] {
        assert!(
            main_rs.contains(required),
            "main.rs should implement CLI surface via {required}"
        );
    }

    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    for required in ["session_needs_extract", "EXTRACTOR_VERSION", "taste_extractions"] {
        assert!(
            extract.contains(required),
            "extract module should implement idempotent CLI semantics via {required}"
        );
    }

    let status = fs::read_to_string("src/taste/status.rs").expect("read status module");
    for required in [
        "sessions_processed",
        "coverage_percent",
        "TasteOutcomeCounts",
        "HIGH_CONFIDENCE_THRESHOLD",
    ] {
        assert!(
            status.contains(required),
            "status module should report CLI coverage counts via {required}"
        );
    }
}

#[test]
fn taste_extraction_scope_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## What we are building",
        "extract taste",
        "D_RL",
        "(context, proposal, outcome)",
        "data-extraction layer",
        "not training a model",
        "notes/command-code-taste-formula.md",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document scope via {required}"
        );
    }

    let formula = fs::read_to_string("notes/command-code-taste-formula.md")
        .expect("read command code taste formula notes");
    for required in [
        "D_RL",
        "(context, proposal, outcome)",
        "jottrace taste",
        "notes/taste-extraction-plan.md",
    ] {
        assert!(
            formula.contains(required),
            "command code taste formula should document jottrace D_RL scope via {required}"
        );
    }

    let design = fs::read_to_string("docs/design.md").expect("read design.md");
    for required in [
        "(context, proposal, outcome)",
        "reward-model training signal",
    ] {
        assert!(
            design.contains(required),
            "docs/design.md should document taste scope via {required}"
        );
    }

    let compiler = fs::read_to_string("src/taste/compiler.rs").expect("read compiler module");
    for required in ["PreferenceExample", "proposal_content", "context", "PreferenceOutcome"] {
        assert!(
            compiler.contains(required),
            "compiler should emit labeled preference rows via {required}"
        );
    }

    let export = fs::read_to_string("src/taste/export.rs").expect("read export module");
    for required in ["context", "chosen", "rejected", "PreferenceOutcome"] {
        assert!(
            export.contains(required),
            "export should map preference rows to trainer triples via {required}"
        );
    }
}

#[test]
fn taste_extraction_implementation_sequence_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## Implementation sequence",
        "1. **Capture real fixtures.**",
        "2. **Shared Claude parse layer.**",
        "3. **Snapshot sidecar resolver.**",
        "4. **`file_timelines` materialization.**",
        "5. **Preference compiler.**",
        "6. **`preference_examples` materialization + CLI.**",
        "7. **Coverage report.**",
        "inline-content snapshots AND",
        "`backupFileName`-referenced snapshots",
        "permission denial",
        "subagent sidechain",
        "normalized `(seq, kind, file_path, content_or_ref,",
        "graceful degradation when",
        "present-at-session-end comparison",
        "coverage (% of file-modifying events resolved)",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document implementation sequence via {required}"
        );
    }

    // Step 1 — fixture corpus markers
    let session = taste_fixture(&format!(
        "claude-cli/projects/-Users-fixture-Workspace-jottrace/{TASTE_SESSION_ID}.jsonl"
    ));
    let session_content = fs::read_to_string(&session).expect("read taste session fixture");
    for required in [
        "\"trackedFileBackups\":[{\"filePath\"",
        "\"backupFileName\"",
        "new_string was NOT written",
        "toolu_taste_edit_revert",
        "taste_subagent.rs",
    ] {
        assert!(
            session_content.contains(required),
            "step 1 fixture corpus should cover {required}"
        );
    }

    // Step 2 — parse layer
    let parse = fs::read_to_string("src/taste/parse.rs").expect("read parse module");
    for required in ["ParsedEvent", "ClaudeSessionParser", "merge_streams"] {
        assert!(
            parse.contains(required),
            "step 2 parse layer should implement {required}"
        );
    }

    // Step 3 — sidecar resolver
    let sidecar = fs::read_to_string("src/taste/sidecar.rs").expect("read sidecar module");
    for required in ["SnapshotSidecarResolver", "backupFileName", "MissingSidecar"] {
        assert!(
            sidecar.contains(required),
            "step 3 sidecar resolver should implement {required}"
        );
    }

    // Step 4 — file_timelines
    assert!(Path::new("src/migrations/010_taste_extraction.sql").exists());
    let timeline = fs::read_to_string("src/taste/timeline.rs").expect("read timeline module");
    assert!(
        timeline.contains("FileTimelineMaterializer"),
        "step 4 should materialize file_timelines via FileTimelineMaterializer"
    );

    // Step 5 — preference compiler
    let compiler = fs::read_to_string("src/taste/compiler.rs").expect("read compiler module");
    for required in ["PreferenceCompiler", "classify_present_at_session_end", "EvidenceKind"] {
        assert!(
            compiler.contains(required),
            "step 5 compiler should implement {required}"
        );
    }

    // Step 6 — preference_examples + CLI
    assert!(Path::new("src/migrations/011_preference_examples.sql").exists());
    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    for required in ["run_taste_extract", "replace_session_preference_examples"] {
        assert!(
            extract.contains(required),
            "step 6 extract should implement {required}"
        );
    }
    let show = fs::read_to_string("src/taste/show.rs").expect("read show module");
    for required in ["run_taste_show_timeline", "run_taste_show_example"] {
        assert!(
            show.contains(required),
            "step 6 show commands should expose {required}"
        );
    }
    let export = fs::read_to_string("src/taste/export.rs").expect("read export module");
    assert!(
        export.contains("run_taste_export"),
        "step 6 export should expose run_taste_export"
    );

    // Step 7 — coverage report
    let status = fs::read_to_string("src/taste/status.rs").expect("read status module");
    for required in ["coverage_percent", "HIGH_CONFIDENCE_THRESHOLD", "run_taste_status"] {
        assert!(
            status.contains(required),
            "step 7 status should report coverage via {required}"
        );
    }
}

#[test]
fn taste_extraction_deferred_r3_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## Deferred: async Task transcripts (R3)",
        "**Status: excluded (2026-06-17).**",
        "tasks/*.output",
        "**Decision: no**",
        "scope for taste-extraction completeness",
        "ships without a",
        "`tasks/*.output` reader",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should document deferred R3 via {required}"
        );
    }

    let inventory =
        fs::read_to_string("docs/reader-source-inventory.md").expect("read reader inventory");
    for required in [
        "tasks/*.output",
        "excluded from taste-extraction scope",
        "notes/taste-extraction-plan.md",
    ] {
        assert!(
            inventory.contains(required),
            "reader inventory should cross-reference deferred R3 via {required}"
        );
    }

    let design = fs::read_to_string("docs/design.md").expect("read design.md");
    assert!(
        design.contains("R3 exception"),
        "design.md should document deferred R3 exception"
    );

    let changelog = fs::read_to_string("CHANGELOG.md").expect("read CHANGELOG.md");
    assert!(
        changelog.contains("tasks/*.output"),
        "CHANGELOG should document R3 tasks/*.output exclusion"
    );

    // No tasks/*.output reader exists in the taste pipeline.
    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    assert!(
        !extract.contains("tasks/") && !extract.contains(".output"),
        "extract module should not implement deferred tasks/*.output reader"
    );
}

#[test]
fn taste_extraction_reference_complete() {
    let plan =
        fs::read_to_string("notes/taste-extraction-plan.md").expect("read taste extraction plan");

    for required in [
        "## Reference",
        "notes/command-code-taste-formula.md",
        "docs/design.md",
        "docs/processor-design.md",
        "src/migrations/001_initial_schema.sql",
    ] {
        assert!(
            plan.contains(required),
            "taste extraction plan should list reference via {required}"
        );
    }

    for path in [
        "notes/command-code-taste-formula.md",
        "docs/design.md",
        "docs/processor-design.md",
        "src/migrations/001_initial_schema.sql",
    ] {
        assert!(
            Path::new(path).exists(),
            "plan reference artifact should exist: {path}"
        );
    }

    let formula = fs::read_to_string("notes/command-code-taste-formula.md")
        .expect("read command code taste formula notes");
    for required in [
        "notes/taste-extraction-plan.md",
        "jottrace taste",
        "D_RL",
    ] {
        assert!(
            formula.contains(required),
            "formula reference should link back to taste extraction via {required}"
        );
    }

    let design = fs::read_to_string("docs/design.md").expect("read design.md");
    for required in [
        "## Taste extraction",
        "notes/taste-extraction-plan.md",
        "sessions",
        "events",
        "file_timelines",
        "preference_examples",
    ] {
        assert!(
            design.contains(required),
            "design.md reference should document taste on base schema via {required}"
        );
    }

    let processor =
        fs::read_to_string("docs/processor-design.md").expect("read processor-design.md");
    for required in [
        "Relationship to taste extraction",
        "src/taste/parse.rs",
        "notes/taste-extraction-plan.md",
    ] {
        assert!(
            processor.contains(required),
            "processor-design reference should document taste boundary via {required}"
        );
    }

    let initial_schema = fs::read_to_string("src/migrations/001_initial_schema.sql")
        .expect("read initial schema migration");
    for required in ["CREATE TABLE sessions", "CREATE TABLE events"] {
        assert!(
            initial_schema.contains(required),
            "initial schema should define base tables taste extraction reads via {required}"
        );
    }

    let extract = fs::read_to_string("src/taste/extract.rs").expect("read extract module");
    for required in [
        "list_parent_claude_sessions",
        "load_merged_session_events",
        "FROM sessions",
    ] {
        assert!(
            extract.contains(required),
            "extract should read from initial-schema tables via {required}"
        );
    }
}
