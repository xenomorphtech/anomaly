;; heartbeat — smallest useful program: emits a signal into the feed and
;; reduces the building's status.
(module
  (import "commander" "signal" (func $signal (param i32 i32)))
  (import "commander" "reduce" (func $reduce (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 16) "heartbeat from wasm module")
  (data (i32.const 64) "{\"op\":\"status\",\"value\":\"wasm-ok\"}")
  (func (export "tick") (result i32)
    (call $signal (i32.const 16) (i32.const 26))
    (call $reduce (i32.const 64) (i32.const 33))
    (i32.const 0)))
