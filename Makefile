PREFIX  ?= /usr
DESTDIR ?=
LIBDIR  := $(DESTDIR)$(PREFIX)/lib/uos

.PHONY: test install deb iso iso-arm64 clean

test:
	python3 -m unittest discover -s tests -t .

install:
	install -d $(LIBDIR)/uos $(DESTDIR)$(PREFIX)/bin \
		$(DESTDIR)$(PREFIX)/lib/binfmt.d $(DESTDIR)$(PREFIX)/share/applications \
		$(DESTDIR)$(PREFIX)/lib/systemd/system
	install -m 644 uos/*.py $(LIBDIR)/uos/
	install -m 755 system/bin/uos $(DESTDIR)$(PREFIX)/bin/uos
	install -m 755 system/bin/uos-binfmt $(LIBDIR)/uos-binfmt
	install -m 755 system/bin/uos-selftest $(LIBDIR)/uos-selftest
	install -m 644 system/systemd/uos-selftest.service $(DESTDIR)$(PREFIX)/lib/systemd/system/
	install -m 644 system/binfmt.d/uos.conf $(DESTDIR)$(PREFIX)/lib/binfmt.d/uos.conf
	install -m 644 system/applications/uos-run.desktop $(DESTDIR)$(PREFIX)/share/applications/

deb:
	./build/build-deb.sh

iso:
	sudo ARCH=amd64 ./build/build-iso.sh

iso-arm64:
	sudo ARCH=arm64 ./build/build-iso.sh

clean:
	rm -rf build/out
