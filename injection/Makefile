all: injection

injection: injection.c
	gcc -shared -fPIC -L../target/x86_64-unknown-linux-gnu/release -pthread -O3 injection.c -o libinjection.so -lclient_injection -ldl

rust-lib: lib.rs
	cargo build --release --target x86_64-unknown-linux-gnu
