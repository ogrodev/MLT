# Local quality gates — mirrors .github/workflows/ci.yml. Run `make check` before pushing.
#
# fmt/clippy are invoked via the rustc-adjacent binaries (portable: works with both the
# Homebrew toolchain locally and rustup in CI), sidestepping any stale ~/.cargo/bin shims.
RUSTC   := $(shell command -v rustc)
RUSTBIN := $(dir $(realpath $(RUSTC)))
FMT     := "$(RUSTBIN)cargo-fmt"
CLIPPY  := "$(RUSTBIN)cargo-clippy"

# Coverage needs LLVM tools; locally we borrow Xcode's, CI uses the llvm-tools-preview component.
LLVM_PROFDATA := $(shell xcrun --find llvm-profdata 2>/dev/null)
LLVM_COV      := $(shell xcrun --find llvm-cov 2>/dev/null)

.PHONY: check fmt fmt-check lint test deny machete purity ui-lint ui-check coverage hooks deps qa qa-release

check: fmt-check lint purity test deny ui-lint ui-check ## all gates (matches CI)

fmt: ; $(FMT) --all
fmt-check: ; $(FMT) --all --check
lint: ; $(CLIPPY) --workspace --all-targets -- -D warnings
test: ; cargo test --workspace
deny: ; cargo deny check
machete: ; cargo machete
purity: ; ./scripts/check-core-purity.sh
ui-lint: ; pnpm exec biome ci .
ui-check: ; pnpm run check
coverage: ; LLVM_PROFDATA="$(LLVM_PROFDATA)" LLVM_COV="$(LLVM_COV)" cargo llvm-cov --package mlt-core --fail-under-lines 80
hooks: ; lefthook install
# Install/refresh dependencies THROUGH Socket Firewall (blocks malware at fetch, any depth).
# Use this instead of bare `cargo fetch` / `pnpm install` when deps change.
deps: ; pnpm exec sfw cargo fetch --locked && pnpm exec sfw pnpm install --frozen-lockfile
# Build + install + launch the app on this Mac for manual QA (debug = fast; qa-release = real build).
qa: ; chmod +x scripts/qa-install.sh && ./scripts/qa-install.sh debug
qa-release: ; chmod +x scripts/qa-install.sh && ./scripts/qa-install.sh release
