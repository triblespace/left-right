[![Codecov](https://codecov.io/github/triblespace/reft-light/coverage.svg?branch=main)](https://codecov.io/gh/triblespace/reft-light)
[![Crates.io](https://img.shields.io/crates/v/reft-light.svg)](https://crates.io/crates/reft-light)
[![Documentation](https://docs.rs/reft-light/badge.svg)](https://docs.rs/reft-light/)

This is a feature-limited version of the
original [left-right](https://github.com/jonhoo/left-right) library.

The original library makes tradeoffs to achieve high performance at the
cost of somewhat convoluted semantics, which makes it difficult to
use with datastructures that have side effects like writing to a file.
This library is designed to be simpler to use, with a focus on
correctness and ease of use, while still providing a high level of
concurrency for reads. It is not intended to be a drop-in replacement
for the original library, but rather a simpler alternative that can be
used in cases where the original library's performance is not
necessary.

# left-right

Left-right is a concurrency primitive for high concurrency reads over a
single-writer data structure. The primitive keeps two copies of the
backing data structure, one that is accessed by readers, and one that is
accessed by the (single) writer. This enables all reads to proceed in
parallel with minimal coordination, and shifts the coordination overhead
to the writer. In the absence of writes, reads scale linearly with the
number of cores.
