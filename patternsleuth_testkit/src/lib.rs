pub use object::SectionKind;
pub use patternsleuth::{
    Memory, MemoryTrait, NamedMemorySection,
    image::{Image, ImageType},
};

pub const BASE: u64 = 0x1_4000_0000;
pub const TEXT: u64 = BASE + 0x1000;
pub const FUNCTION_SIZE: u64 = 0x40;

pub fn section(
    name: &str,
    address: u64,
    kind: SectionKind,
    data: Vec<u8>,
) -> NamedMemorySection<'static> {
    NamedMemorySection::new(name.into(), address, kind, data)
}

fn image(text: Vec<u8>, image_type: ImageType) -> Image<'static> {
    Image::from_memory(
        BASE,
        Memory::from_sections(vec![section(".text", TEXT, SectionKind::Text, text)]),
        image_type,
    )
}

#[cfg(feature = "image-pe")]
pub fn pe(text: Vec<u8>, functions: u32) -> Image<'static> {
    check_layout(&text, functions);
    let mut image = image(
        text,
        ImageType::PEImage(patternsleuth::image::pe::PEImage {
            exception_directory_range: BASE + 0x2000..BASE + 0x2000 + u64::from(functions) * 12,
            exception_children_cache: (0..functions)
                .map(|i| (TEXT + u64::from(i) * FUNCTION_SIZE, Vec::new()))
                .collect(),
        }),
    );
    let pdata = (0..functions)
        .flat_map(|i| [0x1000 + i * 0x40, 0x1040 + i * 0x40, 0x3000 + i * 4])
        .flat_map(u32::to_le_bytes)
        .collect();
    image.memory.sections_mut().extend([
        section(".pdata", BASE + 0x2000, SectionKind::ReadOnlyData, pdata),
        section(
            ".xdata",
            BASE + 0x3000,
            SectionKind::ReadOnlyData,
            [1, 0, 0, 0].repeat(functions as usize),
        ),
    ]);
    image
}

#[cfg(feature = "image-elf")]
pub fn elf(text: Vec<u8>, functions: u32) -> Image<'static> {
    check_layout(&text, functions);
    image(
        text,
        ImageType::ElfImage(patternsleuth::image::elf::ElfImage {
            functions: Some(
                (0..functions)
                    .map(|i| {
                        let start = TEXT + u64::from(i) * FUNCTION_SIZE;
                        start..start + FUNCTION_SIZE
                    })
                    .collect(),
            ),
        }),
    )
}

fn check_layout(text: &[u8], functions: u32) {
    assert!(text.len() <= 0x1000, "text exceeds the fixture code region");
    assert!(
        u64::from(functions) * FUNCTION_SIZE <= 0x1000,
        "function ranges exceed the fixture code region"
    );
}
