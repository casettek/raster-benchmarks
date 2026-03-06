# Web

Zero-dependency, static HTML tools for the `raster-benchmarks` project.

## How to use

**With the local web server (recommended):**

From the repo root:

```
cargo run --manifest-path apps/web-server/Cargo.toml
```

Then open: http://localhost:8010

Use the `PORT` environment variable to change the port:

```
PORT=9000 cargo run --manifest-path apps/web-server/Cargo.toml
```

**Without a server:**

Each tool's `index.html` can also be opened directly in a browser from the filesystem — no build step, no dependencies required.

## Tools

### Settlement Estimator

`settlement-estimator/index.html`

Estimates settlement time, trace size, and DA cost for a Raster program execution, and compares
it against the equivalent zkVM validity proof time. Input your program's zkVM cycle count and
tile execution count (from the Raster toolchain's estimate mode) alongside your desired challenge
window parameters to see the tradeoff between settlement speed and DA cost.

### Scenario Runner

`scenario-runner/index.html`

Runs a workload (e.g. `hello-raster`, `trace-stress`) through the full claim/challenge lifecycle
and shows each step — native execution, trace production, DA availability, claim submission,
challenger replay, and final settlement or slash — with inline performance metrics. Saves
completed runs to browser storage and supports side-by-side comparison of any two past runs.

## Adding new tools

One subdirectory per tool, `index.html` as the entry point. Each tool should be fully
self-contained with no external dependencies so it opens directly from the filesystem.
