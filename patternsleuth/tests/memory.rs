use object::SectionKind;
use patternsleuth::{
    Memory, MemoryTrait, NamedMemorySection,
    image::{Image, ImageType},
};

#[test]
fn owned_sections_preserve_metadata_and_allow_byte_mutation() {
    let mut section =
        NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, vec![0x90, 0xc3]);

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
        NamedMemorySection::new(".data".into(), 0x2000, SectionKind::Data, vec![1]),
        NamedMemorySection::new(".text".into(), 0x1000, SectionKind::Text, vec![2]),
    ]);

    assert_eq!(memory.sections()[0].name(), ".data");
    assert_eq!(memory.sections()[1].name(), ".text");
    assert_eq!(memory.u8(0x1000).unwrap(), 2);

    memory.sections_mut()[1].data_mut()[0] = 3;

    assert_eq!(memory.u8(0x1000).unwrap(), 3);

    memory.sections_mut().push(NamedMemorySection::new(
        ".extra".into(),
        0x3000,
        SectionKind::Data,
        vec![4],
    ));

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

    let mut section =
        NamedMemorySection::new(".empty".into(), 0x1000, SectionKind::Data, Vec::new());

    assert!(section.is_empty());
    assert!(section.data_mut().is_empty());
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
    let mut image = Image::from_memory(0x1000, memory, image_type);

    assert_image_metadata(&image, expected_type);
    assert_image_copy_on_write(&mut image, &bytes);
    assert_image_scans_memory(&image);
}

fn assert_image_metadata(image: &Image<'_>, expected_type: std::mem::Discriminant<ImageType>) {
    assert_eq!(image.base_address, 0x1000);
    assert_eq!(std::mem::discriminant(&image.image_type), expected_type);
    assert!(image.imports.is_empty());
    #[cfg(feature = "symbols")]
    assert!(image.symbols.is_none());
}

fn assert_image_copy_on_write(image: &mut Image<'_>, bytes: &[u8]) {
    assert_eq!(image.memory.sections()[0].data().as_ptr(), bytes.as_ptr());

    image.memory.sections_mut()[0].data_mut()[0] = 0xcc;

    assert_eq!(image.memory.u8(0x1000).unwrap(), 0xcc);
    assert_eq!(bytes[0], 0x90);
}

fn assert_image_scans_memory(image: &Image<'_>) {
    let matches = patternsleuth::resolvers::eval(image, |ctx| {
        Box::pin(async {
            ctx.scan(patternsleuth::scanner::Pattern::new("AA BB").unwrap())
                .await
        })
    });

    assert_eq!(matches, [0x1001]);
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
