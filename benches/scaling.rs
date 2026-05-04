use std::collections::HashSet;
use std::fs;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use tempfile::TempDir;
use waft::config::{CopyStrategy, SymlinkPolicy, WorktreeincludeSemantics};
use waft::error::Result as WaftResult;
use waft::fs::FileSystem;
use waft::git::{GitBackend, GitGix, IgnoreCheckRecord, WorktreeRecord};
use waft::model::{RepoContext, ValidationReport};
use waft::path::RepoRelPath;

const SMALL_SIZES: &[usize] = &[128, 512, 2_048];
const RULE_SIZES: &[usize] = &[16, 64, 256, 1_024];
const DEPTH_SIZES: &[usize] = &[4, 16, 64, 128];
const VALIDATION_SIZES: &[usize] = &[64, 256, 1_024];
const PLANNER_SIZES: &[usize] = &[128, 512, 2_048, 8_192];

fn criterion_config() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(250))
        .measurement_time(Duration::from_secs(1))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn init_git_repo(root: &Path) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["init", "-q"])
        .status()
        .expect("failed to spawn git init");
    assert!(status.success(), "git init failed");
}

fn repo_rel(root: &Path, rel: &str) -> RepoRelPath {
    RepoRelPath::normalize(Path::new(rel), root).unwrap()
}

fn make_deep_rel(index: usize, depth: usize) -> String {
    let mut parts: Vec<String> = (0..depth).map(|i| format!("d{i:03}")).collect();
    parts.push(format!("file_{index:05}.env"));
    parts.join("/")
}

struct RuleCountFixture {
    _tmp: TempDir,
    root: PathBuf,
}

impl RuleCountFixture {
    fn new(rules: usize) -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let mut content = String::new();
        for i in 0..rules.saturating_sub(1) {
            content.push_str(&format!("never_{i:05}.tmp\n"));
        }
        content.push_str("target.env\n");
        write_file(&root.join(".worktreeinclude"), &content);
        Self { _tmp: tmp, root }
    }
}

struct DepthFixture {
    _tmp: TempDir,
    root: PathBuf,
    rel_path: String,
}

impl DepthFixture {
    fn positive(depth: usize) -> Self {
        Self::new(depth, false)
    }

    fn negation_worst_case(depth: usize) -> Self {
        Self::new(depth, true)
    }

    fn new(depth: usize, negation_worst_case: bool) -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();

        write_file(&root.join(".worktreeinclude"), "never_root.tmp\n");

        let mut current = root.clone();
        let mut parts = Vec::new();
        for i in 0..depth {
            parts.push(format!("d{i:03}"));
            current.push(parts.last().unwrap());
            fs::create_dir_all(&current).unwrap();

            let content = if negation_worst_case && i + 1 == depth {
                "never_deep.tmp\n!leaf.env\n"
            } else if negation_worst_case {
                "never_deep.tmp\n"
            } else {
                "*.env\n"
            };
            write_file(&current.join(".worktreeinclude"), content);
        }

        parts.push("leaf.env".to_string());
        Self {
            _tmp: tmp,
            root,
            rel_path: parts.join("/"),
        }
    }
}

struct CandidateFixture {
    _tmp: TempDir,
    root: PathBuf,
}

impl CandidateFixture {
    fn new(files: usize) -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        init_git_repo(&root);
        write_file(&root.join(".worktreeinclude"), "*.env\n");
        write_file(&root.join(".gitignore"), "*.env\n");
        for i in 0..files {
            write_file(&root.join(format!("file_{i:05}.env")), "value\n");
        }
        Self { _tmp: tmp, root }
    }
}

struct ValidationFixture {
    _tmp: TempDir,
    ctx: RepoContext,
}

impl ValidationFixture {
    fn unique_patterns(patterns: usize) -> Self {
        let mut content = String::new();
        for i in 0..patterns {
            content.push_str(&format!("path_{i:05}/file.env\n"));
        }
        Self::with_worktreeinclude(content)
    }

    fn shadowed_negations(patterns: usize) -> Self {
        let mut content = String::new();
        for i in 0..patterns {
            content.push_str(&format!("dir_{i:05}/\n"));
        }
        for i in 0..patterns {
            content.push_str(&format!("!dir_{i:05}/keep.env\n"));
        }
        Self::with_worktreeinclude(content)
    }

    fn with_worktreeinclude(content: String) -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        write_file(&root.join(".worktreeinclude"), &content);
        let ctx = RepoContext {
            source_root: root.clone(),
            dest_root: None,
            main_worktree: root.clone(),
            known_worktrees: vec![root],
            core_ignore_case: false,
        };
        Self { _tmp: tmp, ctx }
    }
}

#[derive(Debug, Default)]
struct NoConfigGit;

impl GitBackend for NoConfigGit {
    fn show_toplevel(&self, _path: &Path) -> WaftResult<PathBuf> {
        unreachable!("validation benchmark only reads config")
    }

    fn list_worktrees(&self, _source_root: &Path) -> WaftResult<Vec<WorktreeRecord>> {
        unreachable!("validation benchmark only reads config")
    }

    fn tracked_paths(
        &self,
        _source_root: &Path,
        _paths: &[RepoRelPath],
    ) -> WaftResult<HashSet<RepoRelPath>> {
        unreachable!("validation benchmark only reads config")
    }

    fn check_ignore(
        &self,
        _source_root: &Path,
        _paths: &[RepoRelPath],
    ) -> WaftResult<Vec<IgnoreCheckRecord>> {
        unreachable!("validation benchmark only reads config")
    }

    fn list_worktreeinclude_candidates(
        &self,
        _source_root: &Path,
        _semantics: WorktreeincludeSemantics,
        _symlink_policy: SymlinkPolicy,
    ) -> WaftResult<Vec<RepoRelPath>> {
        unreachable!("validation benchmark only reads config")
    }

    fn list_ignored_untracked(&self, _source_root: &Path) -> WaftResult<Vec<RepoRelPath>> {
        unreachable!("validation benchmark only reads config")
    }

    fn worktreeinclude_exists_anywhere(
        &self,
        _source_root: &Path,
        _symlink_policy: SymlinkPolicy,
    ) -> WaftResult<bool> {
        unreachable!("validation benchmark only reads config")
    }

    fn read_bool_config(&self, _source_root: &Path, _key: &str) -> WaftResult<bool> {
        Ok(false)
    }

    fn read_config(&self, _source_root: &Path, _key: &str) -> WaftResult<Option<String>> {
        Ok(None)
    }
}

struct PlannerFixture {
    ctx: RepoContext,
    eligible: Vec<RepoRelPath>,
    fs: BenchFs,
    git: EmptyTrackedGit,
}

impl PlannerFixture {
    fn new(paths: usize, depth: usize) -> Self {
        let source_root = PathBuf::from("/tmp/waft-bench/source");
        let dest_root = PathBuf::from("/tmp/waft-bench/dest");
        let mut source_files = HashSet::new();
        let mut eligible = Vec::with_capacity(paths);

        for i in 0..paths {
            let rel = if depth == 0 {
                format!("file_{i:05}.env")
            } else {
                make_deep_rel(i, depth)
            };
            let path = repo_rel(&source_root, &rel);
            source_files.insert(path.to_path(&source_root));
            eligible.push(path);
        }

        let ctx = RepoContext {
            source_root: source_root.clone(),
            dest_root: Some(dest_root),
            main_worktree: source_root,
            known_worktrees: Vec::new(),
            core_ignore_case: false,
        };
        Self {
            ctx,
            eligible,
            fs: BenchFs {
                source_files,
                dest_files: HashSet::new(),
                symlink_dirs: HashSet::new(),
            },
            git: EmptyTrackedGit,
        }
    }
}

#[derive(Debug)]
struct BenchFs {
    source_files: HashSet<PathBuf>,
    dest_files: HashSet<PathBuf>,
    symlink_dirs: HashSet<PathBuf>,
}

impl FileSystem for BenchFs {
    fn exists(&self, path: &Path) -> bool {
        self.source_files.contains(path) || self.dest_files.contains(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.source_files.contains(path) || self.dest_files.contains(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        !self.is_file(path)
    }

    fn is_symlink(&self, path: &Path) -> bool {
        self.symlink_dirs.contains(path)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.is_file(path) {
            Ok(Vec::new())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
        }
    }

    fn parent_has_symlink(&self, path: &Path) -> bool {
        let mut current = path.to_path_buf();
        while let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            if self.symlink_dirs.contains(parent) {
                return true;
            }
            current = parent.to_path_buf();
        }
        false
    }

    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn copy_file(&self, _src: &Path, _dst: &Path, _strategy: CopyStrategy) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct EmptyTrackedGit;

impl GitBackend for EmptyTrackedGit {
    fn show_toplevel(&self, _path: &Path) -> WaftResult<PathBuf> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn list_worktrees(&self, _source_root: &Path) -> WaftResult<Vec<WorktreeRecord>> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn tracked_paths(
        &self,
        _source_root: &Path,
        _paths: &[RepoRelPath],
    ) -> WaftResult<HashSet<RepoRelPath>> {
        Ok(HashSet::new())
    }

    fn check_ignore(
        &self,
        _source_root: &Path,
        _paths: &[RepoRelPath],
    ) -> WaftResult<Vec<IgnoreCheckRecord>> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn list_worktreeinclude_candidates(
        &self,
        _source_root: &Path,
        _semantics: WorktreeincludeSemantics,
        _symlink_policy: SymlinkPolicy,
    ) -> WaftResult<Vec<RepoRelPath>> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn list_ignored_untracked(&self, _source_root: &Path) -> WaftResult<Vec<RepoRelPath>> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn worktreeinclude_exists_anywhere(
        &self,
        _source_root: &Path,
        _symlink_policy: SymlinkPolicy,
    ) -> WaftResult<bool> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn read_bool_config(&self, _source_root: &Path, _key: &str) -> WaftResult<bool> {
        unreachable!("planner benchmark only checks tracked paths")
    }

    fn read_config(&self, _source_root: &Path, _key: &str) -> WaftResult<Option<String>> {
        unreachable!("planner benchmark only checks tracked paths")
    }
}

fn bench_worktreeinclude_rule_count(c: &mut Criterion) {
    let fixtures: Vec<_> = RULE_SIZES
        .iter()
        .copied()
        .map(|size| (size, RuleCountFixture::new(size)))
        .collect();
    let mut group = c.benchmark_group("worktreeinclude_explain_root_rule_count");
    for (rules, fixture) in &fixtures {
        group.throughput(Throughput::Elements(*rules as u64));
        group.bench_with_input(BenchmarkId::from_parameter(rules), fixture, |b, fixture| {
            b.iter(|| {
                black_box(waft::worktreeinclude::explain(
                    black_box(&fixture.root),
                    black_box("target.env"),
                    false,
                    false,
                    SymlinkPolicy::Follow,
                ))
            });
        });
    }
    group.finish();
}

fn bench_worktreeinclude_depth(c: &mut Criterion) {
    let positive_fixtures: Vec<_> = DEPTH_SIZES
        .iter()
        .copied()
        .map(|depth| (depth, DepthFixture::positive(depth)))
        .collect();
    let negation_fixtures: Vec<_> = DEPTH_SIZES
        .iter()
        .copied()
        .map(|depth| (depth, DepthFixture::negation_worst_case(depth)))
        .collect();

    let mut positive = c.benchmark_group("worktreeinclude_explain_nested_depth_positive");
    for (depth, fixture) in &positive_fixtures {
        positive.throughput(Throughput::Elements(*depth as u64));
        positive.bench_with_input(BenchmarkId::from_parameter(depth), fixture, |b, fixture| {
            b.iter(|| {
                black_box(waft::worktreeinclude::explain(
                    black_box(&fixture.root),
                    black_box(&fixture.rel_path),
                    false,
                    false,
                    SymlinkPolicy::Follow,
                ))
            });
        });
    }
    positive.finish();

    let mut negation = c.benchmark_group("worktreeinclude_explain_nested_depth_negation_scan");
    for (depth, fixture) in &negation_fixtures {
        negation.throughput(Throughput::Elements(*depth as u64));
        negation.bench_with_input(BenchmarkId::from_parameter(depth), fixture, |b, fixture| {
            b.iter(|| {
                black_box(waft::worktreeinclude::explain(
                    black_box(&fixture.root),
                    black_box(&fixture.rel_path),
                    false,
                    false,
                    SymlinkPolicy::Follow,
                ))
            });
        });
    }
    negation.finish();
}

fn bench_candidate_enumeration(c: &mut Criterion) {
    let fixtures: Vec<_> = SMALL_SIZES
        .iter()
        .copied()
        .map(|files| (files, CandidateFixture::new(files)))
        .collect();
    let backend = GitGix::new();

    let mut group = c.benchmark_group("candidate_enumeration_gix_file_count");
    for (files, fixture) in &fixtures {
        group.throughput(Throughput::Elements(*files as u64));
        group.bench_with_input(BenchmarkId::from_parameter(files), fixture, |b, fixture| {
            b.iter(|| {
                let candidates = backend
                    .list_worktreeinclude_candidates(
                        black_box(&fixture.root),
                        WorktreeincludeSemantics::Git,
                        SymlinkPolicy::Follow,
                    )
                    .unwrap();
                black_box(candidates.len())
            });
        });
    }
    group.finish();
}

fn bench_validation(c: &mut Criterion) {
    let unique_fixtures: Vec<_> = VALIDATION_SIZES
        .iter()
        .copied()
        .map(|patterns| (patterns, ValidationFixture::unique_patterns(patterns)))
        .collect();
    let shadowed_fixtures: Vec<_> = VALIDATION_SIZES
        .iter()
        .copied()
        .map(|patterns| (patterns, ValidationFixture::shadowed_negations(patterns)))
        .collect();
    let git = NoConfigGit;

    let mut unique = c.benchmark_group("validate_unique_rule_count");
    for (patterns, fixture) in &unique_fixtures {
        unique.throughput(Throughput::Elements(*patterns as u64));
        unique.bench_with_input(
            BenchmarkId::from_parameter(patterns),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(waft::validate::validate(
                        black_box(&fixture.ctx),
                        &git,
                        SymlinkPolicy::Follow,
                    ))
                });
            },
        );
    }
    unique.finish();

    let mut shadowed = c.benchmark_group("validate_shadowed_negation_rule_count");
    for (patterns, fixture) in &shadowed_fixtures {
        shadowed.throughput(Throughput::Elements(*patterns as u64));
        shadowed.bench_with_input(
            BenchmarkId::from_parameter(patterns),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(waft::validate::validate(
                        black_box(&fixture.ctx),
                        &git,
                        SymlinkPolicy::Follow,
                    ))
                });
            },
        );
    }
    shadowed.finish();
}

fn bench_planner(c: &mut Criterion) {
    let flat_fixtures: Vec<_> = PLANNER_SIZES
        .iter()
        .copied()
        .map(|paths| (paths, PlannerFixture::new(paths, 0)))
        .collect();
    let deep_fixtures: Vec<_> = SMALL_SIZES
        .iter()
        .copied()
        .map(|paths| (paths, PlannerFixture::new(paths, 32)))
        .collect();

    let mut flat = c.benchmark_group("planner_missing_dest_file_count_flat");
    for (paths, fixture) in &flat_fixtures {
        flat.throughput(Throughput::Elements(*paths as u64));
        flat.bench_with_input(BenchmarkId::from_parameter(paths), fixture, |b, fixture| {
            b.iter_batched(
                || fixture.eligible.clone(),
                |eligible| {
                    let plan = waft::planner::plan(
                        black_box(&fixture.ctx),
                        ValidationReport::default(),
                        black_box(eligible),
                        &fixture.git,
                        &fixture.fs,
                        false,
                        true,
                    )
                    .unwrap();
                    black_box(plan.entries.len())
                },
                BatchSize::LargeInput,
            );
        });
    }
    flat.finish();

    let mut deep = c.benchmark_group("planner_missing_dest_file_count_depth_32");
    for (paths, fixture) in &deep_fixtures {
        deep.throughput(Throughput::Elements(*paths as u64));
        deep.bench_with_input(BenchmarkId::from_parameter(paths), fixture, |b, fixture| {
            b.iter_batched(
                || fixture.eligible.clone(),
                |eligible| {
                    let plan = waft::planner::plan(
                        black_box(&fixture.ctx),
                        ValidationReport::default(),
                        black_box(eligible),
                        &fixture.git,
                        &fixture.fs,
                        false,
                        true,
                    )
                    .unwrap();
                    black_box(plan.entries.len())
                },
                BatchSize::LargeInput,
            );
        });
    }
    deep.finish();
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets =
        bench_worktreeinclude_rule_count,
        bench_worktreeinclude_depth,
        bench_candidate_enumeration,
        bench_validation,
        bench_planner
}
criterion_main!(benches);
