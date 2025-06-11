use std::fmt::{Debug, Display};

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{impl_resolver, Result},
    MemoryTrait,
};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct AESKeys(Vec<AESKey>);
impl Debug for AESKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AESKeys ")?;
        f.debug_list().entries(self.0.iter()).finish()
    }
}

#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct AESKey([u8; 32]);
impl Display for AESKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
impl Debug for AESKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self, f)
    }
}

impl_resolver!(collect, AESKeys);
impl_resolver!(PEImage, AESKeys, |ctx| async {
    #[derive(Debug, Clone, Copy)]
    enum KeyType {
        A,
        B,
        C,
    }
    let patterns = [
        (KeyType::A, "C7 45 D0 ?? ?? ?? ?? C7 45 D4 ?? ?? ?? ?? ?? ?? ?? ?? C7 45 D8 ?? ?? ?? ?? C7 45 DC ?? ?? ?? ?? C7 45 E0 ?? ?? ?? ?? C7 45 E4 ?? ?? ?? ?? C7 45 E8 ?? ?? ?? ?? C7 45 EC ?? ?? ?? ??"),
        (KeyType::B, "C7 01 ?? ?? ?? ?? C7 41 04 ?? ?? ?? ?? C7 41 08 ?? ?? ?? ?? C7 41 0C ?? ?? ?? ?? C7 41 10 ?? ?? ?? ?? C7 41 14 ?? ?? ?? ?? C7 41 18 ?? ?? ?? ?? C7 41 1C ?? ?? ?? ?? C3"),
        (KeyType::C, "C7 45 D0 ?? ?? ?? ?? C7 45 D4 ?? ?? ?? ?? C7 45 D8 ?? ?? ?? ?? C7 45 DC ?? ?? ?? ?? 0F 10 45 D0 C7 45 E0 ?? ?? ?? ?? C7 45 E4 ?? ?? ?? ?? C7 45 E8 ?? ?? ?? ?? C7 45 EC ?? ?? ?? ??"),
    ];

    let res = join_all(
        patterns
            .iter()
            .map(|(tag, p)| ctx.scan_tagged(tag, Pattern::new(p).unwrap())),
    )
    .await;

    let mem = &ctx.image().memory;

    let extract_key = |key_type: KeyType, addresses: Vec<usize>| {
        addresses.into_iter().map(move |addr| -> Result<_> {
            let mut key = [0; 32];
            match key_type {
                KeyType::A => {
                    let data = &mem.range(addr..addr + 60)?;
                    (key[0..4]).copy_from_slice(&data[3..7]);
                    (key[4..8]).copy_from_slice(&data[10..14]);
                    (key[8..12]).copy_from_slice(&data[21..25]);
                    (key[12..16]).copy_from_slice(&data[28..32]);
                    (key[16..20]).copy_from_slice(&data[35..39]);
                    (key[20..24]).copy_from_slice(&data[42..46]);
                    (key[24..28]).copy_from_slice(&data[49..53]);
                    (key[28..32]).copy_from_slice(&data[56..60]);
                }
                KeyType::B => {
                    let data = &mem.range(addr..addr + 55)?;
                    (key[0..4]).copy_from_slice(&data[2..6]);
                    (key[4..8]).copy_from_slice(&data[9..13]);
                    (key[8..12]).copy_from_slice(&data[16..20]);
                    (key[12..16]).copy_from_slice(&data[23..27]);
                    (key[16..20]).copy_from_slice(&data[30..34]);
                    (key[20..24]).copy_from_slice(&data[37..41]);
                    (key[24..28]).copy_from_slice(&data[44..48]);
                    (key[28..32]).copy_from_slice(&data[51..55]);
                }
                KeyType::C => {
                    let data = &mem.range(addr..addr + 60)?;
                    (key[0..4]).copy_from_slice(&data[3..7]);
                    (key[4..8]).copy_from_slice(&data[10..14]);
                    (key[8..12]).copy_from_slice(&data[17..21]);
                    (key[12..16]).copy_from_slice(&data[24..28]);
                    (key[16..20]).copy_from_slice(&data[35..39]);
                    (key[20..24]).copy_from_slice(&data[42..46]);
                    (key[24..28]).copy_from_slice(&data[49..53]);
                    (key[28..32]).copy_from_slice(&data[56..60]);
                }
            };
            Ok(if key == *b"\x6f\x16\x80\x73\xb9\xb2\x14\x49\xd7\x42\x24\x17\x00\x06\x8a\xda\xbc\x30\x6f\xa9\xaa\x38\x31\x16\x4d\xee\x8d\xe3\x4e\x0e\xfb\xb0" {
                None
            } else {
                Some(AESKey(key))
            })
        })
    };

    Ok(Self(
        res.into_iter()
            .flat_map(|(key, _, addresses)| extract_key(*key, addresses))
            .flat_map(|i| i.transpose())
            .collect::<Result<Vec<_>>>()?,
    ))
});

impl_resolver!(ElfImage, AESKeys, |_ctx| async {
    super::bail_out!("ElfImage unimplemented");
});
