.PHONY: hooks fmt clippy test check

# Install git pre-commit hook
hooks:
	@cp scripts/pre-commit .git/hooks/pre-commit
	@chmod +x .git/hooks/pre-commit
	@echo "Pre-commit hook installed."

# Individual CI-equivalent targets
fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace --all-targets

# Run all checks (same as CI)
check: fmt-check clippy test
