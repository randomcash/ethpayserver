(module
  (memory (export "memory") 1)
  (data (i32.const 1024) "{\"type\":\"section\",\"title\":\"Fixture\",\"children\":[{\"type\":\"text\",\"text\":\"rendered by a plugin\",\"style\":\"body\"},{\"type\":\"fields\",\"fields\":[{\"label\":\"Source\",\"value\":\"wasm\"}]}]}")
  (func (export "alloc") (param $len i32) (result i32)
    (i32.const 4096))
  (func (export "render_page") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.const 1024) (i64.const 32))
      (i64.const 173)))
)
