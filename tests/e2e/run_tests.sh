#!/bin/bash
# End-to-end test runner for cc1
# Usage: ./tests/e2e/run_tests.sh

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CC1="${SCRIPT_DIR}/../../target/release/cc1"
LLC="llc-18"
GCC="gcc"
PASS=0
FAIL=0
SKIP=0

# Check prerequisites
if ! command -v "$LLC" &>/dev/null; then
    echo "SKIP: $LLC not found"
    exit 0
fi

if ! "$GCC" -m32 -x c -o /dev/null /dev/null 2>/dev/null; then
    echo "NOTE: -m32 not available, using x86_64 target"
    GCC_FLAGS=""
    CC1_FLAGS="-m64"
else
    GCC_FLAGS="-m32"
    CC1_FLAGS=""
fi

# Build release binary
echo "Building cc1..."
(cd "$SCRIPT_DIR/../.." && cargo build --release 2>&1) || { echo "FAIL: cargo build failed"; exit 1; }

run_test() {
    local src="$1"
    local expected="$2"
    local name="$(basename "$src" .c)"
    local ll="/tmp/cc1_test_${name}.ll"
    local asm="/tmp/cc1_test_${name}.s"
    local bin="/tmp/cc1_test_${name}"

    # Compile C -> LLVM IR
    if ! "$CC1" $CC1_FLAGS "$src" -o "$ll" 2>/dev/null; then
        echo "FAIL: $name (cc1 failed)"
        FAIL=$((FAIL + 1))
        return
    fi

    # LLVM IR -> Assembly
    if ! "$LLC" "$ll" -o "$asm" 2>/dev/null; then
        echo "FAIL: $name (llc failed)"
        FAIL=$((FAIL + 1))
        return
    fi

    # Assembly -> Binary
    if ! "$GCC" $GCC_FLAGS -o "$bin" "$asm" 2>/dev/null; then
        echo "FAIL: $name (gcc link failed)"
        FAIL=$((FAIL + 1))
        return
    fi

    # Run binary
    set +e
    "$bin"
    local actual=$?
    set -e

    if [ "$actual" -eq "$expected" ]; then
        echo "PASS: $name (exit=$actual)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (expected=$expected, got=$actual)"
        FAIL=$((FAIL + 1))
    fi

    # Cleanup
    rm -f "$ll" "$asm" "$bin"
}

echo ""
echo "=== cc1 End-to-End Tests ==="
echo ""

# Test cases: source file, expected exit code
run_test "$SCRIPT_DIR/return42.c"    42
run_test "$SCRIPT_DIR/arithmetic.c"  16
run_test "$SCRIPT_DIR/ifelse.c"      1
run_test "$SCRIPT_DIR/loop.c"        55
run_test "$SCRIPT_DIR/forloop.c"     10
run_test "$SCRIPT_DIR/funcall.c"     42
run_test "$SCRIPT_DIR/fib.c"         55
run_test "$SCRIPT_DIR/global.c"      13
run_test "$SCRIPT_DIR/pointer.c"     99
run_test "$SCRIPT_DIR/dowhile.c"     10
run_test "$SCRIPT_DIR/nested_if.c"   30
run_test "$SCRIPT_DIR/factorial.c"   120
run_test "$SCRIPT_DIR/divmod.c"      31
run_test "$SCRIPT_DIR/ternary.c"     100

echo ""
echo "=== Results: $PASS passed, $FAIL failed, $SKIP skipped ==="

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
