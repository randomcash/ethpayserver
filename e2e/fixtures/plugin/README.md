# The e2e fixture plugin

A plugin that exists only to prove the machinery carries a page from a wasm
module to a browser. The real plugin this machinery was built for is private,
so a public suite cannot use it — and should not: what belongs here is the
*mechanism*, which lives in this repository. What the mechanism is used for is
tested where that lives.

`fixture.wasm` is the smallest module the host accepts. It exports `memory`,
the `alloc` the host writes its argument through, and a `render_page` that
answers with one fixed page. It reads nothing, so it has no behaviour of its
own to get wrong — a fixture that computed its answer would be a second
implementation to debug whenever a test failed.

`fixture.wat` is the source those bytes were compiled from, checked in beside
them so the binary is not opaque. To regenerate:

    wat2wasm fixture.wat -o fixture.wasm

Any wat2wasm will do; the `wat` crate in payserver-commons is the one used
here, since this repository already depends on it.
