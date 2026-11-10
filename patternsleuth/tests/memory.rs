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
    assert_eq!(section.len(), 2);
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
    assert_eq!(memory.u8(0x2000).unwrap(), 1);
    assert_eq!(memory.u8(0x1000).unwrap(), 2);
    assert!(memory.u8(0x3000).is_err());
}

#[test]
fn empty_memory_has_no_sections_or_readable_addresses() {
    let memory = Memory::from_sections(Vec::new());

    assert!(memory.sections().is_empty());
    assert!(memory.u8(0x1000).is_err());
}

#[test]
fn empty_section_preserves_empty_data() {
    let section = NamedMemorySection::new(".empty".into(), 0x1000, SectionKind::Data, Vec::new());

    assert!(section.is_empty());
    assert!(section.data().is_empty());
}

fn check_image_from_memory(image_type: ImageType) {
    let bytes = [0x90, 0xaa, 0xbb, 0xc3];
    let memory = Memory::from_sections(vec![NamedMemorySection::new(
        ".text".into(),
        0x1000,
        SectionKind::Text,
        &bytes[..],
    )]);
    let expected_type = std::mem::discriminant(&image_type);
    let image = Image::from_memory(0x1000, memory, image_type);

    assert_image_metadata(&image, expected_type);
    assert_image_memory(&image, &bytes);
}

fn assert_image_metadata(image: &Image<'_>, expected_type: std::mem::Discriminant<ImageType>) {
    assert_eq!(image.base_address, 0x1000);
    assert_eq!(std::mem::discriminant(&image.image_type), expected_type);
    assert!(image.imports.is_empty());
    #[cfg(feature = "symbols")]
    assert!(image.symbols.is_none());
}

fn assert_image_memory(image: &Image<'_>, bytes: &[u8]) {
    assert_eq!(image.memory.sections().len(), 1);
    assert_eq!(image.memory.sections()[0].address(), 0x1000);
    assert_eq!(image.memory.sections()[0].data().as_ptr(), bytes.as_ptr());
    assert_eq!(image.memory.sections()[0].data(), bytes);
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
    let mut image = Image::from_memory(0x1000, memory, image_type);
    image
        .imports
        .insert("module".into(), [("function".into(), 0x3000)].into());
    image
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
    assert_eq!(image.imports, original.imports);
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
    assert_eq!(image.memory.u8(0x2000).unwrap(), 42);
    assert!(image.memory.u8(0x1000).is_err());
    assert_eq!(
        original.without_section("missing").memory.sections().len(),
        2
    );
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
    assert_eq!(image.base_address, original.base_address);
    assert_eq!(image.imports, original.imports);
}

#[test]
fn with_text_rejects_missing_text_sections() {
    let image = test_image().without_section(".text");
    let result = image.with_text(vec![0xcc]);

    assert!(matches!(
        result,
        Err(patternsleuth::MemoryAccessError::MemoryOutOfBoundsError)
    ));
    assert!(image.memory.sections().is_empty());
}

#[test]
fn copy_sections_returns_owned_independent_data_in_order() {
    let image = test_image().with_section(NamedMemorySection::new(
        ".data".into(),
        0x2000,
        SectionKind::Data,
        vec![42],
    ));
    let sections = image.copy_sections();

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].name(), ".text");
    assert_eq!(sections[0].data(), b"original");
    assert_ne!(
        sections[0].data().as_ptr(),
        image.memory.sections()[0].data().as_ptr()
    );
    assert_eq!(sections[1].address(), 0x2000);
    assert_eq!(sections[1].kind(), SectionKind::Data);
    assert_eq!(sections[1].data(), [42]);

    drop(image);
    let memory = Memory::from_sections(sections);

    assert_eq!(memory.u8(0x1000).unwrap(), b'o');
    assert_eq!(memory.u8(0x2000).unwrap(), 42);
}

#[cfg(feature = "image-pe")]
#[test]
fn construct_pe_image_from_borrowed_memory() {
    check_image_from_memory(ImageType::PEImage(patternsleuth::image::pe::PEImage {
        exception_directory_range: 0..0,
        exception_children_cache: Default::default(),
    }));
}

#[cfg(feature = "image-elf")]
#[test]
fn construct_elf_image_from_borrowed_memory() {
    check_image_from_memory(ImageType::ElfImage(patternsleuth::image::elf::ElfImage {
        functions: Some(Vec::new()),
    }));
}
