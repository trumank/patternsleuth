use patternsleuth_testkit::{Memory, MemoryTrait, NamedMemorySection, SectionKind, section};

#[test]
fn owned_sections_preserve_metadata_and_allow_byte_mutation() {
    let mut section = section(".text", 0x1000, SectionKind::Text, vec![0x90, 0xc3]);
    assert_eq!(section.name(), ".text");
    assert_eq!(section.address(), 0x1000);
    assert_eq!(section.kind(), SectionKind::Text);
    assert_eq!(section.len(), 2);
    section.data_mut()[1] = 0xcc;
    assert_eq!(section.data(), [0x90, 0xcc]);
}

#[test]
fn borrowed_sections_copy_on_mutation_without_changing_the_source() {
    let bytes = [0x90, 0xc3];
    let mut section =
        NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, &bytes[..]);
    assert_eq!(section.data().as_ptr(), bytes.as_ptr());
    section.data_mut()[0] = 0xcc;
    assert_eq!(section.data(), [0xcc, 0xc3]);
    assert_eq!(bytes, [0x90, 0xc3]);
    assert_ne!(section.data().as_ptr(), bytes.as_ptr());
}

#[test]
fn memory_preserves_section_order_and_supports_add_remove_and_mutate() {
    let mut memory = Memory::from_sections(vec![
        section(".data", 0x2000, SectionKind::Data, vec![1]),
        section(".text", 0x1000, SectionKind::Text, vec![2]),
    ]);
    assert_eq!(memory.sections()[0].name(), ".data");
    assert_eq!(memory.sections()[1].name(), ".text");
    assert_eq!(memory.u8(0x1000).unwrap(), 2);
    memory.sections_mut()[1].data_mut()[0] = 3;
    assert_eq!(memory.u8(0x1000).unwrap(), 3);

    memory
        .sections_mut()
        .push(section(".extra", 0x3000, SectionKind::Data, vec![4]));
    assert_eq!(memory.u8(0x3000).unwrap(), 4);
    memory
        .sections_mut()
        .retain(|section| section.name() != ".extra");
    assert!(memory.u8(0x3000).is_err());
}

#[test]
fn empty_memory_and_sections_are_valid() {
    let memory = Memory::from_sections(Vec::new());
    assert!(memory.sections().is_empty());
    assert!(memory.u8(0x1000).is_err());
    let mut section = section(".empty", 0x1000, SectionKind::Data, Vec::new());
    assert!(section.is_empty());
    assert!(section.data_mut().is_empty());
}
