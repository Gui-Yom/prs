all: setup server

setup:
	echo Checking Rust installation for current user
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh
	chmod +x rustup.sh
	./rustup.sh -y
	source "$HOME/.cargo/env"

server-trace:
	cargo build -q --release --features trace

server:
	cargo build -q --release --features tracing/release_max_level_debug

server2:
	cargo build -q --release --features client2,tracing/release_max_level_debug
