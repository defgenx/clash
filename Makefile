.PHONY: build install clean

build:
	cargo build --release
ifeq ($(shell uname),Darwin)
	codesign --force --sign - target/release/clash
endif

install: build
	cp target/release/clash /usr/local/bin/clash

clean:
	cargo clean
