//! Manual parity harness comparing `waft copy`, `wt step copy-ignored`, and
//! `claude --worktree` behavior around `.worktreeinclude`.
//!
//! Run with:
//!   cargo test --test worktrunk_parity -- --ignored --nocapture

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    RootSimple,
    NoWorktreeinclude,
    NestedOverride,
    NestedAnchored,
    CrossFileNegationCaveat,
    NestedWorktreeInRepo,
    ToolStateDir,
    #[cfg(unix)]
    SymlinkedWorktreeinclude,
}

impl Scenario {
    fn all() -> Vec<Self> {
        let mut scenarios = vec![
            Self::RootSimple,
            Self::NoWorktreeinclude,
            Self::NestedOverride,
            Self::NestedAnchored,
            Self::CrossFileNegationCaveat,
            Self::NestedWorktreeInRepo,
            Self::ToolStateDir,
        ];
        #[cfg(unix)]
        scenarios.push(Self::SymlinkedWorktreeinclude);
        scenarios
    }

    fn id(self) -> &'static str {
        match self {
            Self::RootSimple => "root-simple",
            Self::NoWorktreeinclude => "no-worktreeinclude",
            Self::NestedOverride => "nested-worktreeinclude-override",
            Self::NestedAnchored => "nested-anchored-pattern",
            Self::CrossFileNegationCaveat => "cross-file-negation-caveat",
            Self::NestedWorktreeInRepo => "nested-worktree-in-repo",
            Self::ToolStateDir => "tool-state-directory",
            #[cfg(unix)]
            Self::SymlinkedWorktreeinclude => "symlinked-worktreeinclude",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::RootSimple => "Root .worktreeinclude selecting one ignored file.",
            Self::NoWorktreeinclude => "No .worktreeinclude present; only .gitignore rules exist.",
            Self::NestedOverride => {
                "Nested .worktreeinclude negates a shallow include for a subdirectory."
            }
            Self::NestedAnchored => {
                "Anchored pattern (`/foo`) inside nested .worktreeinclude file."
            }
            Self::CrossFileNegationCaveat => {
                "Root include selects directory; nested negation attempts to remove child."
            }
            Self::NestedWorktreeInRepo => {
                "Nested linked worktree located at .worktrees/ under the source repo."
            }
            Self::ToolStateDir => {
                "Tool-state directory (.conductor/) explicitly selected by .worktreeinclude."
            }
            #[cfg(unix)]
            Self::SymlinkedWorktreeinclude => "Symlinked .worktreeinclude file at repo root.",
        }
    }
}

#[derive(Debug)]
struct PreparedCase {
    _tempdir: TempDir,
    main: PathBuf,
    linked: PathBuf,
    wt_config: PathBuf,
}

#[derive(Debug)]
struct ToolRun {
    success: bool,
    copied_paths: BTreeSet<String>,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct ScenarioResult {
    scenario: Scenario,
    waft: ToolRun,
    worktrunk: ToolRun,
    claude: ToolRun,
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed in {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.email", "test@test.com"]);
    git(path, &["config", "user.name", "Test"]);
    write_file(path, "README.md", "test\n");
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-m", "init"]);
}

fn commit_rules(main: &Path, paths: &[&str]) {
    if paths.is_empty() {
        return;
    }
    let mut args = vec!["add", "-f"];
    args.extend_from_slice(paths);
    git(main, &args);
    git(main, &["commit", "-m", "rules"]);
}

fn prepare_case(scenario: Scenario) -> PreparedCase {
    let tempdir = TempDir::new().unwrap();
    let main = tempdir.path().join("main");
    let linked = tempdir.path().join("linked");

    init_repo(&main);

    match scenario {
        Scenario::RootSimple => {
            write_file(&main, ".gitignore", ".env\n");
            write_file(&main, ".worktreeinclude", ".env\n");
            commit_rules(&main, &[".gitignore", ".worktreeinclude"]);
        }
        Scenario::NoWorktreeinclude => {
            write_file(&main, ".gitignore", ".env\ncache/\n");
            commit_rules(&main, &[".gitignore"]);
        }
        Scenario::NestedOverride => {
            write_file(&main, ".gitignore", "*.env\n");
            write_file(&main, ".worktreeinclude", "*.env\n");
            write_file(&main, "config/.worktreeinclude", "!*.env\n");
            commit_rules(
                &main,
                &[".gitignore", ".worktreeinclude", "config/.worktreeinclude"],
            );
        }
        Scenario::NestedAnchored => {
            write_file(&main, ".gitignore", "foo\nconfig/foo\n");
            write_file(&main, "config/.worktreeinclude", "/foo\n");
            commit_rules(&main, &[".gitignore", "config/.worktreeinclude"]);
        }
        Scenario::CrossFileNegationCaveat => {
            write_file(&main, ".gitignore", "secrets/\n");
            write_file(&main, ".worktreeinclude", "secrets/\n");
            write_file(&main, "secrets/.worktreeinclude", "!private.key\n");
            commit_rules(
                &main,
                &[".gitignore", ".worktreeinclude", "secrets/.worktreeinclude"],
            );
        }
        Scenario::NestedWorktreeInRepo => {
            write_file(&main, ".gitignore", ".worktrees/\n");
            write_file(&main, ".worktreeinclude", ".worktrees/**/*.env\n");
            commit_rules(&main, &[".gitignore", ".worktreeinclude"]);
        }
        Scenario::ToolStateDir => {
            write_file(&main, ".gitignore", ".conductor/\n");
            write_file(&main, ".worktreeinclude", ".conductor/**/*.key\n");
            commit_rules(&main, &[".gitignore", ".worktreeinclude"]);
        }
        #[cfg(unix)]
        Scenario::SymlinkedWorktreeinclude => {
            write_file(&main, ".gitignore", ".env\n");
            write_file(&main, "real.wti", ".env\n");
            std::os::unix::fs::symlink("real.wti", main.join(".worktreeinclude")).unwrap();
            commit_rules(&main, &[".gitignore", "real.wti", ".worktreeinclude"]);
        }
    }

    git(
        &main,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "feature"],
    );

    match scenario {
        Scenario::RootSimple => {
            write_file(&main, ".env", "ROOT_SECRET=1\n");
        }
        Scenario::NoWorktreeinclude => {
            write_file(&main, ".env", "ROOT_SECRET=1\n");
            write_file(&main, "cache/build.bin", "cache\n");
        }
        Scenario::NestedOverride => {
            write_file(&main, "root.env", "root\n");
            write_file(&main, "config/sub.env", "sub\n");
        }
        Scenario::NestedAnchored => {
            write_file(&main, "foo", "root\n");
            write_file(&main, "config/foo", "nested\n");
        }
        Scenario::CrossFileNegationCaveat => {
            write_file(&main, "secrets/private.key", "private\n");
        }
        Scenario::NestedWorktreeInRepo => {
            let nested = main.join(".worktrees/nested");
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    nested.to_str().unwrap(),
                    "-b",
                    "nested-edge",
                ],
            );
            write_file(&nested, ".env", "nested\n");
        }
        Scenario::ToolStateDir => {
            write_file(&main, ".conductor/state/dev.key", "tool-state\n");
        }
        #[cfg(unix)]
        Scenario::SymlinkedWorktreeinclude => {
            write_file(&main, ".env", "ROOT_SECRET=1\n");
        }
    }

    let wt_config = tempdir.path().join("wt-config.toml");
    fs::write(&wt_config, "").unwrap();

    PreparedCase {
        _tempdir: tempdir,
        main,
        linked,
        wt_config,
    }
}

fn ignored_untracked_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|e| format!("failed to run git ls-files: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "git ls-files failed in {}\nstdout:\n{}\nstderr:\n{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output
        .stdout
        .split(|&b| b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect())
}

fn intersect_with_source(source: &BTreeSet<String>, dest: BTreeSet<String>) -> BTreeSet<String> {
    dest.into_iter().filter(|p| source.contains(p)).collect()
}

fn find_worktrunk_bin() -> Option<String> {
    if let Ok(path) = std::env::var("WORKTRUNK_BIN") {
        let output = Command::new(&path).arg("--version").output().ok()?;
        if output.status.success() {
            return Some(path);
        }
    }

    let output = Command::new("wt").arg("--version").output().ok()?;
    if output.status.success() {
        Some("wt".to_string())
    } else {
        None
    }
}

fn find_claude_bin() -> Option<String> {
    if let Ok(path) = std::env::var("CLAUDE_BIN") {
        let output = Command::new(&path).arg("--version").output().ok()?;
        if output.status.success() {
            return Some(path);
        }
    }

    let output = Command::new("claude").arg("--version").output().ok()?;
    if output.status.success() {
        Some("claude".to_string())
    } else {
        None
    }
}

fn run_waft(case: &PreparedCase) -> ToolRun {
    let source_paths = ignored_untracked_paths(&case.main).unwrap_or_default();

    let output = Command::new(env!("CARGO_BIN_EXE_waft"))
        .args([
            "copy",
            "--source",
            case.main.to_str().unwrap(),
            "--dest",
            case.linked.to_str().unwrap(),
            "--overwrite",
        ])
        .output()
        .unwrap();

    let copied_paths = ignored_untracked_paths(&case.linked)
        .map(|paths| intersect_with_source(&source_paths, paths))
        .unwrap_or_default();

    ToolRun {
        success: output.status.success(),
        copied_paths,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn run_worktrunk(case: &PreparedCase, worktrunk_bin: &str) -> ToolRun {
    let source_paths = ignored_untracked_paths(&case.main).unwrap_or_default();

    let output = Command::new(worktrunk_bin)
        .args([
            "--config",
            case.wt_config.to_str().unwrap(),
            "-C",
            case.main.to_str().unwrap(),
            "step",
            "copy-ignored",
            "--from",
            "main",
            "--to",
            "feature",
            "--force",
        ])
        .env(
            "WORKTRUNK_SYSTEM_CONFIG_PATH",
            case.main.join("does-not-exist"),
        )
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    let copied_paths = ignored_untracked_paths(&case.linked)
        .map(|paths| intersect_with_source(&source_paths, paths))
        .unwrap_or_default();

    ToolRun {
        success: output.status.success(),
        copied_paths,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn extract_first_backtick_path(text: &str) -> Option<PathBuf> {
    let mut pieces = text.split('`');
    while pieces.next().is_some() {
        let candidate = pieces.next()?.trim();
        if candidate.starts_with('/') {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

fn run_claude(case: &PreparedCase, claude_bin: &str) -> ToolRun {
    let source_paths = ignored_untracked_paths(&case.main).unwrap_or_default();

    let output = Command::new(claude_bin)
        .current_dir(&case.main)
        .args([
            "--worktree",
            "--model",
            "haiku",
            "-p",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--output-format",
            "text",
            "State the full path of your current working directory (CWD).",
        ])
        .output()
        .unwrap();

    let mut success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut copied_paths = BTreeSet::new();

    if success {
        match extract_first_backtick_path(&stdout) {
            Some(worktree_path) => {
                match ignored_untracked_paths(&worktree_path) {
                    Ok(paths) => {
                        copied_paths = intersect_with_source(&source_paths, paths);
                    }
                    Err(e) => {
                        success = false;
                        if !stderr.is_empty() {
                            stderr.push('\n');
                        }
                        stderr.push_str(&format!(
                            "failed to inspect claude-created worktree {}: {}",
                            worktree_path.display(),
                            e
                        ));
                    }
                }

                // Cleanup the temporary claude-created worktree.
                let cleanup = Command::new("git")
                    .arg("-C")
                    .arg(&case.main)
                    .args([
                        "worktree",
                        "remove",
                        "--force",
                        worktree_path.to_str().unwrap(),
                    ])
                    .output();

                if let Ok(cleanup_out) = cleanup
                    && !cleanup_out.status.success()
                {
                    if !stderr.is_empty() {
                        stderr.push('\n');
                    }
                    stderr.push_str(&format!(
                        "warning: cleanup failed for {}\nstdout:\n{}\nstderr:\n{}",
                        worktree_path.display(),
                        String::from_utf8_lossy(&cleanup_out.stdout),
                        String::from_utf8_lossy(&cleanup_out.stderr)
                    ));
                }
            }
            None => {
                success = false;
                if !stderr.is_empty() {
                    stderr.push('\n');
                }
                stderr.push_str(&format!(
                    "failed to parse claude worktree path from output:\n{}",
                    stdout
                ));
            }
        }
    }

    ToolRun {
        success,
        copied_paths,
        stdout,
        stderr,
    }
}

fn outcomes_equal(a: &ToolRun, b: &ToolRun) -> bool {
    a.success == b.success && a.copied_paths == b.copied_paths
}

fn format_paths(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        "(none)".to_string()
    } else {
        paths.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn status_cell(run: &ToolRun) -> String {
    if run.success {
        format!("success ({})", run.copied_paths.len())
    } else {
        "failure".to_string()
    }
}

fn agreement_note(waft: &ToolRun, worktrunk: &ToolRun, claude: &ToolRun) -> String {
    let ww = outcomes_equal(waft, worktrunk);
    let wc = outcomes_equal(waft, claude);
    let tc = outcomes_equal(worktrunk, claude);

    if ww && wc && tc {
        return "all three agree".to_string();
    }

    let mut pairs = Vec::new();
    if ww {
        pairs.push("waft=wt");
    }
    if wc {
        pairs.push("waft=claude");
    }
    if tc {
        pairs.push("wt=claude");
    }

    if pairs.is_empty() {
        "no pair agrees".to_string()
    } else {
        pairs.join(", ")
    }
}

fn build_report(results: &[ScenarioResult], worktrunk_bin: &str, claude_bin: &str) -> String {
    let mut report = String::new();

    let all_three_agree = results
        .iter()
        .filter(|r| outcomes_equal(&r.waft, &r.worktrunk) && outcomes_equal(&r.waft, &r.claude))
        .count();
    let waft_wt_agree = results
        .iter()
        .filter(|r| outcomes_equal(&r.waft, &r.worktrunk))
        .count();
    let waft_claude_agree = results
        .iter()
        .filter(|r| outcomes_equal(&r.waft, &r.claude))
        .count();
    let wt_claude_agree = results
        .iter()
        .filter(|r| outcomes_equal(&r.worktrunk, &r.claude))
        .count();

    report.push_str("# Parity Report (waft vs wt vs claude)\n\n");
    report.push_str("This report compares copied ignored-file outcomes across three tools over the same scenario matrix.\n\n");
    report.push_str("- Harness: `cargo test --test worktrunk_parity -- --ignored --nocapture`\n");
    report.push_str(&format!("- worktrunk binary: `{}`\n", worktrunk_bin));
    report.push_str(&format!("- claude binary: `{}`\n", claude_bin));
    report.push_str(&format!("- Scenarios: {}\n", results.len()));
    report.push_str(&format!("- All three agree: {}\n", all_three_agree));
    report.push_str(&format!("- Pairwise agree (waft=wt): {}\n", waft_wt_agree));
    report.push_str(&format!(
        "- Pairwise agree (waft=claude): {}\n",
        waft_claude_agree
    ));
    report.push_str(&format!(
        "- Pairwise agree (wt=claude): {}\n\n",
        wt_claude_agree
    ));

    report
        .push_str("| Scenario | waft | wt | claude | waft=wt | waft=claude | wt=claude | Note |\n");
    report.push_str("|---|---|---|---|---|---|---|---|\n");

    for result in results {
        let waft_wt = if outcomes_equal(&result.waft, &result.worktrunk) {
            "yes"
        } else {
            "no"
        };
        let waft_claude = if outcomes_equal(&result.waft, &result.claude) {
            "yes"
        } else {
            "no"
        };
        let wt_claude = if outcomes_equal(&result.worktrunk, &result.claude) {
            "yes"
        } else {
            "no"
        };

        report.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            result.scenario.id(),
            status_cell(&result.waft),
            status_cell(&result.worktrunk),
            status_cell(&result.claude),
            waft_wt,
            waft_claude,
            wt_claude,
            agreement_note(&result.waft, &result.worktrunk, &result.claude)
        ));
    }

    report.push_str("\n## Details\n\n");

    for result in results {
        report.push_str(&format!("### `{}`\n\n", result.scenario.id()));
        report.push_str(&format!("{}\n\n", result.scenario.description()));
        report.push_str(&format!(
            "- Agreement: {}\n",
            agreement_note(&result.waft, &result.worktrunk, &result.claude)
        ));
        report.push_str(&format!(
            "- waft copied: {}\n",
            format_paths(&result.waft.copied_paths)
        ));
        report.push_str(&format!(
            "- wt copied: {}\n",
            format_paths(&result.worktrunk.copied_paths)
        ));
        report.push_str(&format!(
            "- claude copied: {}\n",
            format_paths(&result.claude.copied_paths)
        ));

        if !result.waft.success {
            report.push_str("- waft stderr:\n```");
            report.push('\n');
            report.push_str(result.waft.stderr.trim());
            report.push_str("\n```\n");
        }

        if !result.worktrunk.success {
            report.push_str("- wt stderr:\n```");
            report.push('\n');
            report.push_str(result.worktrunk.stderr.trim());
            report.push_str("\n```\n");
        }

        if !result.claude.success {
            report.push_str("- claude stderr:\n```");
            report.push('\n');
            report.push_str(result.claude.stderr.trim());
            report.push_str("\n```\n");
            if !result.claude.stdout.trim().is_empty() {
                report.push_str("- claude stdout:\n```");
                report.push('\n');
                report.push_str(result.claude.stdout.trim());
                report.push_str("\n```\n");
            }
        }

        report.push('\n');
    }

    report
}

#[test]
#[ignore = "manual parity harness; requires wt and claude CLIs"]
fn generate_worktrunk_parity_report() {
    let worktrunk_bin = match find_worktrunk_bin() {
        Some(bin) => bin,
        None => {
            eprintln!(
                "skipping parity harness: `wt` not found (set WORKTRUNK_BIN or install worktrunk)"
            );
            return;
        }
    };

    let claude_bin = match find_claude_bin() {
        Some(bin) => bin,
        None => {
            eprintln!(
                "skipping parity harness: `claude` not found (set CLAUDE_BIN or install Claude Code)"
            );
            return;
        }
    };

    let mut results = Vec::new();

    for scenario in Scenario::all() {
        let waft_case = prepare_case(scenario);
        let wt_case = prepare_case(scenario);
        let claude_case = prepare_case(scenario);

        let waft_run = run_waft(&waft_case);
        let worktrunk_run = run_worktrunk(&wt_case, &worktrunk_bin);
        let claude_run = run_claude(&claude_case, &claude_bin);

        results.push(ScenarioResult {
            scenario,
            waft: waft_run,
            worktrunk: worktrunk_run,
            claude: claude_run,
        });
    }

    let report = build_report(&results, &worktrunk_bin, &claude_bin);

    let report_path = PathBuf::from(".context/worktrunk_parity_report.md");
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&report_path, &report).unwrap();

    println!("{}", report);
    println!("report written to {}", report_path.display());

    assert!(
        !results.is_empty(),
        "expected parity harness to run at least one scenario"
    );
}
