;; http_probe — polls an https endpoint each tick (within the http budget) and
;; reduces a task's state to done/blocked depending on reachability.
(module
  (import "commander" "http" (func $http (param i32 i32 i32 i32 i32 i32) (result i64)))
  (import "commander" "signal" (func $signal (param i32 i32)))
  (import "commander" "reduce" (func $reduce (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "GET")
  (data (i32.const 8) "https://example.com/")
  (data (i32.const 40) "http probe: endpoint reachable")
  (data (i32.const 80) "http probe: request FAILED")
  (data (i32.const 120) "{\"op\":\"task\",\"title\":\"http probe\",\"state\":\"done\"}")
  (data (i32.const 200) "{\"op\":\"task\",\"title\":\"http probe\",\"state\":\"blocked\"}")
  (func (export "tick") (result i32)
    (if (i64.gt_s
          (call $http (i32.const 0) (i32.const 3) (i32.const 8) (i32.const 20)
                      (i32.const 0) (i32.const 0))
          (i64.const 0))
      (then
        (call $signal (i32.const 40) (i32.const 30))
        (call $reduce (i32.const 120) (i32.const 49)))
      (else
        (call $signal (i32.const 80) (i32.const 26))
        (call $reduce (i32.const 200) (i32.const 52))))
    (i32.const 0)))
