

# TODO
- [ ] dynamic struct members
- [ ] checked class casting
- [ ] inheritance/downcasting
- [ ] property access
- [ ] ObjectRef from *const UObject
- [ ] object creation
- [ ] UFunction calling

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
