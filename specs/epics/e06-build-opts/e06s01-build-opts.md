# e06s01: Release profile and native build verification

## §1 Business narrative
As a user on a 4GB ARM tablet, I need a small, fast-cold-start binary that uses all my CPU's instructions.

## §5 Main flow
1. [profile.release]: lto="thin", codegen-units=1, panic="abort", strip=true
2. install.sh: RUSTFLAGS="-C target-cpu=native" cargo build --release
3. tokio features trimmed from "full" to concrete set
4. Result: 8.4MB -> 5.7MB (-32%), faster execution

## §6 Constraints
- Binary is NOT portable (target-cpu=native) — rebuild per machine
- panic=abort safe (no catch_unwind in codebase)
- LTO increases compile time (one-time cost)

## §17 Gherkin
```gherkin
Scenario: Stripped binary
  Given a release build with strip=true
  When I run cargo build --release
  Then the binary is <= 6MB

Scenario: Native build script
  Given install.sh with target-cpu=native
  When I run ./install.sh
  Then the binary is installed to ~/.cargo/bin/dewey
```

## §18 Out of scope
- Cross-compilation tooling
- Binary patching / UPX compression
- SIMD-optimized ZIP reading
