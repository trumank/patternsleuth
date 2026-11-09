use patternsleuth::{resolvers::eval, scanner::Pattern};
use patternsleuth_testkit::{BASE, FUNCTION_SIZE, Image, MemoryTrait, TEXT};

fn check_image(image: Image<'_>) {
    assert_eq!(image.base_address, BASE);
    assert_eq!(image.memory.u8(TEXT).unwrap(), 0x90);
    assert!(image.imports.is_empty());
    let mut ranges = image.get_root_functions().unwrap();
    ranges.sort_by_key(|range| range.start);
    assert_eq!(
        ranges,
        [
            TEXT..TEXT + FUNCTION_SIZE,
            TEXT + FUNCTION_SIZE..TEXT + 2 * FUNCTION_SIZE
        ]
    );
    let first = image
        .get_root_function(TEXT + FUNCTION_SIZE - 1)
        .unwrap()
        .unwrap();
    assert_eq!(first.range, TEXT..TEXT + FUNCTION_SIZE);
    let second = image
        .get_root_function(TEXT + FUNCTION_SIZE)
        .unwrap()
        .unwrap();
    assert_eq!(second.range, TEXT + FUNCTION_SIZE..TEXT + 2 * FUNCTION_SIZE);

    let matches = eval(&image, |ctx| {
        Box::pin(async { ctx.scan(Pattern::new("AA BB").unwrap()).await })
    });
    assert_eq!(matches, [TEXT + 1]);
}

#[cfg(feature = "image-pe")]
#[test]
fn pe_fixture_has_readable_functions_unwind_records_and_scannable_bytes() {
    let image = patternsleuth_testkit::pe(vec![0x90, 0xaa, 0xbb, 0xc3], 2);
    assert_eq!(
        image
            .memory
            .sections()
            .iter()
            .map(|section| section.name())
            .collect::<Vec<_>>(),
        [".text", ".pdata", ".xdata"]
    );
    assert_eq!(
        image.get_root_function(TEXT).unwrap().unwrap().unwind,
        BASE + 0x3000
    );
    check_image(image);
}

#[cfg(feature = "image-elf")]
#[test]
fn elf_fixture_has_function_ranges_and_scannable_bytes() {
    let image = patternsleuth_testkit::elf(vec![0x90, 0xaa, 0xbb, 0xc3], 2);
    assert_eq!(image.memory.sections().len(), 1);
    check_image(image);
}

#[test]
fn fixtures_allow_empty_function_tables() {
    #[cfg(feature = "image-pe")]
    assert!(
        patternsleuth_testkit::pe(Vec::new(), 0)
            .get_root_functions()
            .unwrap()
            .is_empty()
    );
    #[cfg(feature = "image-elf")]
    assert!(
        patternsleuth_testkit::elf(Vec::new(), 0)
            .get_root_functions()
            .unwrap()
            .is_empty()
    );
}

#[test]
#[should_panic(expected = "text exceeds the fixture code region")]
fn fixtures_reject_overlapping_text() {
    #[cfg(feature = "image-pe")]
    patternsleuth_testkit::pe(vec![0; 0x1001], 0);
    #[cfg(all(feature = "image-elf", not(feature = "image-pe")))]
    patternsleuth_testkit::elf(vec![0; 0x1001], 0);
}

#[test]
#[should_panic(expected = "function ranges exceed the fixture code region")]
fn fixtures_reject_overlapping_function_ranges() {
    #[cfg(feature = "image-pe")]
    patternsleuth_testkit::pe(Vec::new(), 65);
    #[cfg(all(feature = "image-elf", not(feature = "image-pe")))]
    patternsleuth_testkit::elf(Vec::new(), 65);
}
