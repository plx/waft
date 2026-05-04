# Scaling Benchmarks

These benchmarks are local characterization tools, not CI gates. They are
intended to make size-sensitive behavior obvious enough to spot accidentally
quadratic or otherwise unfavorable scaling before optimizing.

Run them with:

```sh
cargo bench --bench scaling
```

Criterion writes detailed reports under `target/criterion/`. The benchmark
groups cover:

- `.worktreeinclude` explanation as rule count grows.
- `.worktreeinclude` explanation as nested rule depth grows.
- A negation-heavy nested case that exercises ancestor checks.
- Default gix candidate enumeration as file count grows.
- Validation of large rule files, including duplicate/shadowed-negation lint
  scans.
- Copy planning as eligible path count and path depth grow.
