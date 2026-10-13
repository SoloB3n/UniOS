PREFIX  ?= /usr
DESTDIR ?=
LIBDIR  := $(DESTDIR)$(PREFIX)/lib/uos
BIN     ?= target/release/uos

.PHONY: build test lint install deb iso iso-arm64 clean

build:
	cargo build --release --locked

test:
	cargo test --locked

lint:
	cargo fmt --check
	cargo clippy --all-targets --locked -- -D warnings

# Run "make build" first (as a normal user), then "sudo make install".
install:
	@test -x $(BIN) || { echo "$(BIN) is missing: run 'make build' first" >&2; exit 1; }
	install -d $(LIBDIR) $(DESTDIR)$(PREFIX)/bin \
		$(DESTDIR)$(PREFIX)/lib/binfmt.d $(DESTDIR)$(PREFIX)/share/applications \
		$(DESTDIR)$(PREFIX)/lib/systemd/system
	install -m 755 $(BIN) $(DESTDIR)$(PREFIX)/bin/uos
	install -m 755 system/bin/uos-binfmt $(LIBDIR)/uos-binfmt
	install -m 755 system/bin/uos-selftest $(LIBDIR)/uos-selftest
	install -m 644 system/binfmt.d/uos.conf $(DESTDIR)$(PREFIX)/lib/binfmt.d/uos.conf
	install -m 644 system/applications/uos-run.desktop $(DESTDIR)$(PREFIX)/share/applications/
	install -m 644 system/systemd/uos-selftest.service $(DESTDIR)$(PREFIX)/lib/systemd/system/

deb:
	./build/build-deb.sh

iso:
	sudo ARCH=amd64 ./build/build-iso.sh

iso-arm64:
	sudo ARCH=arm64 ./build/build-iso.sh

clean:
	cargo clean
	rm -rf build/out
