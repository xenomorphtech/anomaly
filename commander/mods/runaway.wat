;; runaway — spins forever; exists to demonstrate the fuel budget trapping a tick.
(module
  (memory (export "memory") 1)
  (func (export "tick") (result i32)
    (loop $spin (br $spin))
    (i32.const 0)))
