use object::SectionKind;
use patternsleuth::{
    Memory, MemoryTrait, NamedMemorySection,
    image::{Image, ImageType},
};

#[test]
fn owned_sections_preserve_metadata_and_bytes() {
    let section =
        NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, vec![0x90, 0xc3]);

    assert_eq!(section.name(), ".text");
    assert_eq!(section.address(), 0x1000);
    assert_eq!(section.kind(), SectionKind::Text);
    assert_eq!(section.data(), [0x90, 0xc3]);
}

#[test]
fn borrowed_sections_preserve_source_storage() {
    let bytes = [0x90, 0xc3];
    let section = NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, &bytes[..]);

    assert_eq!(section.data().as_ptr(), bytes.as_ptr());
    assert_eq!(section.data(), bytes);
}

#[test]
fn memory_preserves_section_order_and_bytes() {
    let memory = Memory::from_sections(vec![
        NamedMemorySection::new(".data".into(), 0x2000, SectionKind::Data, vec![1]),
        NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, vec![2]),
    ]);

    assert_eq!(memory.sections()[0].name(), ".data");
    assert_eq!(memory.sections()[1].name(), ".text");
    assert_eq!(memory.u8(0x1000).unwrap(), 2);
}

#[test]
fn image_from_memory_preserves_input_and_initializes_metadata() {
    let image = test_image();

    assert_eq!(image.base_address, 0x1000);
    assert_eq!(image.memory.sections()[0].data(), b"original");
    assert!(image.imports.is_empty());
    #[cfg(feature = "symbols")]
    assert!(image.symbols.is_none());

    match image.image_type {
        #[cfg(feature = "image-pe")]
        ImageType::PEImage(pe) => assert_eq!(pe.exception_directory_range, 0x2000..0x200c),
        #[cfg(feature = "image-elf")]
        ImageType::ElfImage(elf) => assert_eq!(elf.functions.unwrap(), [0x1000..0x1040]),
    }
}

fn test_image() -> Image<'static> {
    #[cfg(feature = "image-pe")]
    let image_type = ImageType::PEImage(patternsleuth::image::pe::PEImage {
        exception_directory_range: 0x2000..0x200c,
        exception_children_cache: Default::default(),
    });
    #[cfg(not(feature = "image-pe"))]
    let image_type = ImageType::ElfImage(patternsleuth::image::elf::ElfImage {
        functions: Some(vec![0x1000..0x1040]),
    });
    let memory = Memory::from_sections(vec![NamedMemorySection::new(
        ".text".into(),
        0x1000,
        SectionKind::Text,
        &b"original"[..],
    )]);
    Image::from_memory(0x1000, memory, image_type)
}

#[test]
fn with_section_appends_without_changing_the_original() {
    let original = test_image();
    let image = original.with_section(NamedMemorySection::new(
        ".data".into(),
        0x2000,
        SectionKind::Data,
        vec![42],
    ));

    assert_eq!(original.memory.sections().len(), 1);
    assert_eq!(image.memory.sections().len(), 2);
    assert_eq!(image.memory.sections()[0].name(), ".text");
    assert_eq!(image.memory.u8(0x2000).unwrap(), 42);
}

#[test]
fn without_section_removes_named_sections_without_changing_the_original() {
    let original = test_image().with_section(NamedMemorySection::new(
        ".data".into(),
        0x2000,
        SectionKind::Data,
        vec![42],
    ));
    let image = original.without_section(".text");

    assert_eq!(original.memory.sections().len(), 2);
    assert_eq!(image.memory.sections().len(), 1);
    assert_eq!(image.memory.sections()[0].name(), ".data");
}

#[test]
fn with_text_replaces_only_bytes_without_changing_the_original() {
    let original = test_image();
    let bytes = [0xcc, 0xc3];
    let image = original.with_text(&bytes[..]).unwrap();
    let text = &image.memory.sections()[0];

    assert_eq!(original.memory.sections()[0].data(), b"original");
    assert_eq!(text.name(), ".text");
    assert_eq!(text.address(), 0x1000);
    assert_eq!(text.kind(), SectionKind::Text);
    assert_eq!(text.data(), bytes);
    assert_eq!(text.data().as_ptr(), bytes.as_ptr());
}

#[test]
fn with_text_rejects_missing_text_sections() {
    let image = test_image().without_section(".text");
    let result = image.with_text(vec![0xcc]);

    assert!(matches!(
        result,
        Err(patternsleuth::MemoryAccessError::MemoryOutOfBoundsError)
    ));
}

#[test]
fn copy_sections_returns_owned_independent_data_in_order() {
    let sections = {
        let bytes = vec![42];
        let image = test_image().with_section(NamedMemorySection::new(
            ".data".into(),
            0x2000,
            SectionKind::Data,
            &bytes[..],
        ));
        image.copy_sections()
    };

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].name(), ".text");
    assert_eq!(sections[0].data(), b"original");
    assert_eq!(sections[1].name(), ".data");
    assert_eq!(sections[1].address(), 0x2000);
    assert_eq!(sections[1].kind(), SectionKind::Data);
    assert_eq!(sections[1].data(), [42]);
}
