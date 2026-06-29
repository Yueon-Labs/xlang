.PHONY: check ast-if c-loop c-array-type run-array-literal run-if run-option-demo run-result-demo run-compound run-struct-demo run-print-demo run-array-sum run-array-double clean

check:
	cargo run --quiet --bin xlangc -- check examples/*.x

ast-if:
	cargo run --quiet --bin xlangc -- ast examples/if_else.x

c-loop:
	cargo run --quiet --bin xlangc -- c examples/loop.x
	cc -c build/loop.c -o build/loop.o

c-array-type:
	cargo run --quiet --bin xlangc -- c examples/array_type.x
	cc -c build/array_type.c -o build/array_type.o

run-array-literal:
	cargo run --quiet --bin xlangc -- run examples/array_literal.x

run-if:
	cargo run --quiet --bin xlangc -- run examples/if_else.x

run-option-demo:
	cargo run --quiet --bin xlangc -- run examples/option_demo.x

run-result-demo:
	cargo run --quiet --bin xlangc -- run examples/result_demo.x

run-compound:
	cargo run --quiet --bin xlangc -- run examples/compound.x

run-struct-demo:
	cargo run --quiet --bin xlangc -- run examples/struct_demo.x

run-print-demo:
	cargo run --quiet --bin xlangc -- run examples/print_demo.x

run-array-sum:
	cargo run --quiet --bin xlangc -- run examples/array_sum.x

run-array-double:
	cargo run --quiet --bin xlangc -- run examples/array_double.x

clean:
	rm -rf build target
