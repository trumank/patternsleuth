// modified version of sift4 crate to operate directly on bytes
pub fn simple(s1: &[u8], s2: &[u8]) -> i32 {
    return sift4_offset(s1, s2, 5);
}

fn min_usize(u1: usize, u2: usize) -> usize {
    if u1 <= u2 {
        u1
    } else {
        u2
    }
}

fn max_usize(u1: usize, u2: usize) -> usize {
    if u1 >= u2 {
        u1
    } else {
        u2
    }
}

fn sift4_offset(s1: &[u8], s2: &[u8], max_offset: usize) -> i32 {
    let l1 = s1.len();
    let l2 = s2.len();

    // handle empty strings
    if l1 == 0 {
        if l2 == 0 {
            return 0;
        } else {
            return l2 as i32;
        }
    }

    if l2 == 0 {
        return l1 as i32;
    }

    let mut c1 = 0; // cursor for string 1
    let mut c2 = 0; // cursor for string 2
    let mut lcss = 0; // largest common subsequence
    let mut local_cs = 0; // local common substring

    while c1 < l1 && c2 < l2 {
        if s1[c1] == s2[c2] {
            local_cs += 1;
        } else {
            lcss += local_cs;
            local_cs = 0;
            if c1 != c2 {
                c1 = min_usize(c1, c2);
                c2 = c1; // using min allows the computation of transpositions
            }

            for i in 0..max_offset {
                if (c1 + 1 < l1 || c2 + i < l2) == false {
                    break;
                }

                if c1 + i < l1 && s1[c1 + i] == s2[c2] {
                    c1 += i;
                    local_cs += 1;
                    break;
                }
                if (c2 + i < l2) && (s1[c1] == s2[c2 + i]) {
                    c2 += i;
                    local_cs += 1;
                    break;
                }
            }
        }
        c1 += 1;
        c2 += 1;
    }
    lcss += local_cs;
    (max_usize(l1, l2) - lcss) as i32
}
