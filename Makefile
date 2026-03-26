.PHONY: build install clean

build:
	cargo build --release
ifeq ($(shell uname),Darwin)
	codesign --force --sign - target/release/clash
endif

install: build
	rm -f /usr/local/bin/clash
	cp target/release/clash /usr/local/bin/clash
ifeq ($(shell uname),Darwin)
	codesign --force --sign - /usr/local/bin/clash
endif

clean:
	cargo clean
