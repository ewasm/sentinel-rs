(module
  (type (;0;) (func))
  (type (;1;) (func (param i32 i32) (result i32)))
  (import "env" "foo" (func (;0;) (type 0)))
  (import "env" "boo" (func (;1;) (type 0)))
  (func (;2;) (type 1) (param i32 i32) (result i32)
    call 0
    call 1
    get_local 0
    get_local 1
    i32.add)
  (func (;3;) (type 1) (param i32 i32) (result i32)
    get_local 0
    get_local 1
    get_global 0
    i32.const 2
    i32.add
    set_global 0
    get_global 0
    i32.const 1024
    i32.gt_u
    if  ;; label = @1
      unreachable
    end
    call 2
    get_global 0
    i32.const 2
    i32.sub
    set_global 0)
  (global (;0;) (mut i32) (i32.const 0))
  (export "i32.add" (func 3)))
