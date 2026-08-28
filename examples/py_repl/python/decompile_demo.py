# Decompile a live Blueprint class's Kismet bytecode.
#   python repl.py --script decompile_demo.py [ClassPath]
#
# Finds a loaded Blueprint-generated class, decompiles every function to readable
# text, prints it, and demonstrates the two things the bytecode APIs are for:
#   1. cls.decompile()      -> readable source (structured control flow)
#   2. func.disassemble()   -> the structured instruction tree
#   3. mapping a bytecode offset back to a source line (the ue.trace bridge)
#
# Pass a class path to target it; otherwise the first Blueprint class with script
# bytecode is used. Everything runs on the game thread and is a point-in-time
# snapshot -- see UE_API.txt "KISMET BYTECODE".

import sys

import ue


def first_blueprint_class():
    """The first loaded UClass that has at least one function with bytecode."""
    for cls in ue.objects_of_class("/Script/Engine.BlueprintGeneratedClass"):
        for fn in cls.functions():
            if fn.script:
                return cls
    return None


def main():
    if len(sys.argv) > 1:
        cls = ue.find(sys.argv[1])
        if cls is None:
            print(f"class not found: {sys.argv[1]}", file=sys.stderr)
            return
    else:
        cls = first_blueprint_class()
        if cls is None:
            print("no Blueprint class with bytecode is loaded", file=sys.stderr)
            return

    print(f"# {cls._path}\n", file=sys.stderr)

    # 1. Whole-class decompile. One function failing to parse doesn't sink the
    #    rest -- it shows up in that function's status.
    src = cls.decompile(structured=True, offsets=False)
    for f in src.functions():
        if not f["status"].startswith(("structured", "flat")):
            print(f"  ! {f['name']}: {f['status']}", file=sys.stderr)
    print(src.text)

    # 2. Disassemble one function into its instruction tree.
    fn = next((f for f in cls.functions() if f.script), None)
    if fn is not None:
        print(f"\n# disassembly of {fn._name}", file=sys.stderr)
        for ins in fn.disassemble()[:20]:
            detail = ""
            # Surface a resolved reference if this instruction has one.
            for v in ins.fields.values():
                if isinstance(v, ue.KismetRef) and v.path:
                    detail = f"  -> {v.path}"
                    break
            print(f"  0x{ins.offset:04x}  {ins.op}{detail}", file=sys.stderr)

        # 3. Map a bytecode offset to a source line, the same offset ue.trace
        #    reports -- so a trace event can point at a line of the decompile.
        di = next((f for f in src.functions() if f["name"] == fn._name), None)
        # 'index' is the decl index line_of_offset wants -- not the position in
        # functions(), which an unresolved function (index None) shifts.
        if di is not None and di["index"] is not None:
            line = src.line_of_offset(di["index"], 0)
            if line is not None:
                print(f"\n# {fn._name} entry (offset 0) -> line {line[0]}", file=sys.stderr)


main()
