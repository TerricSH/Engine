# Gate 19 Session Prompts

Gate 19 builds release engineering. Do not add new gameplay features here.

## Session 19A: Build Packaging Owner

Goal: Implement reproducible build scripts, asset packaging, platform packages.

Owns:
- build/package scripts
- release metadata generation

Must not edit:
- runtime gameplay systems except packaging hooks

Expected output:
- desktop/mobile package artifacts at agreed scope

Validation:
- tagged build dry run

## Session 19B: Profiling QA Owner

Goal: Implement profiling capture, automated scene runner, and regression thresholds.

Owns:
- profiling and QA automation modules/scripts

Must not edit:
- production code to game performance numbers without review

Expected output:
- CPU/GPU/memory baselines
- automated scene validation

Validation:
- CI or local QA run report

## Session 19C: CI Diagnostics Release Owner

Goal: Implement CI workflows, artifact archive, crash diagnostics, symbols, checksums.

Owns:
- CI workflow files
- diagnostics export scripts
- artifact archive layout

Must not edit:
- gameplay/runtime systems

Expected output:
- signed/checksumed artifacts, symbols, diagnostic bundles

Validation:
- release candidate dry run
