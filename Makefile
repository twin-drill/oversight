.PHONY: install test lint check clean uninstall e2e

install:
	bash setup.sh

test:
	cargo test

lint:
	cargo clippy -- -D warnings

check:
	cargo check

clean:
	cargo clean

uninstall:
	bash uninstall.sh --yes

e2e:
	@cargo build --release && \
	TEMP_HOME=$$(mktemp -d) && \
	HOME=$$TEMP_HOME \
	PATH="$$(pwd)/target/release:$$PATH" \
	OVERSIGHT_BIN=oversight \
	WORKSPACE=$$(pwd) \
	bash tests/e2e/ci.sh; \
	STATUS=$$?; \
	rm -rf $$TEMP_HOME; \
	exit $$STATUS
