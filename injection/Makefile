all: injection

injection: injection.c rust-lib
	gcc -shared -fPIC -L../target/x86_64-unknown-linux-gnu/release -pthread injection.c -o libinjection.so -lclient_injection -ldl

rust-lib: lib.rs
	cargo build -q --release --target x86_64-unknown-linux-gnu
