

# TODO
- [ ] dynamic struct members
- [x] checked class casting
- [x] inheritance/downcasting
  - [ ] need to figure out how accept base class or anything that inherits from it
- [x] property access
  - [ ] impl more prop types
    - [ ] FArrayProperty
    - [ ] FMapProperty
    - [ ] FSetProperty
    - [ ] object properties...
    - [ ] struct properties...
  - [ ] need to figure out props share address space (FBoolProperty)
- [ ] ObjectRef from *const UObject
- [ ] object creation
- [ ] UFunction calling
- [ ] kismet disassembly
- [ ] kismet hooking

## random ideas:

wrap every property in Cell? how does that work for containers such as TArray?

### casts:

```rust
// works for classes in `class_cast_flags`. what about others?
obj->cast::<UFunction>()
obj->is::<UFunction>()

// nvm blueprint classes won't exist as concrete types anyway:
obj->cast(my_blueprint_class) // returns ??;
obj->is(my_blueprint_class); // this needs to work
````


### inheritance

traits?
macros?
deref?


```rust
trait UObjectTrait {
  fn get_class(&self) -> *const UClass;
  fn get_path(&self) -> String;
}

trait UStructTrait : UObjectTrait {
  fn get_script(&self) -> &TArray<u8>;
}
```
