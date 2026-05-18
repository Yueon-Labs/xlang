.PHONY: check ast-if run-if clean

check:
	cargo run --quiet --bin xlangc -- check examples/*.x

ast-if:
	cargo run --quiet --bin xlangc -- ast examples/if_else.x

run-if:
	cargo run --quiet --bin xlangc -- run examples/if_else.x

clean:
	rm -rf build target
