all: server

setup:
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh
	chmod +x rustup.sh
	./rustup.sh -y

server-trace:
	cargo build --release --features trace

server:
	cargo build --release --features tracing/release_max_level_debug
