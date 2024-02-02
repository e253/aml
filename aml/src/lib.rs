#![cfg_attr(feature = "stdsimd", feature(stdsimd))]

#[cfg(test)]
mod tests;

#[cfg(target_arch = "x86")]
use core::arch::x86;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use rayon::prelude::*;

pub struct F32Tensor<'a> {
    shape: Vec<usize>,
    data: &'a [f32],
}

impl<'a> F32Tensor<'a> {
    /// Utility Method eliminating footguns assoc. with creating tensors by hand
    pub fn new(data: &'a [f32], shape: Vec<usize>) -> F32Tensor<'a> {
        assert!(shape.len() == 2, "Only Shapes of length 2 are supported");
        assert!(
            shape[0] % 16 == 0,
            "Dim 0 {} must be divisible by 16",
            shape[0]
        );
        assert!(
            shape[1] % 16 == 0,
            "Dim 1 {} must be divisible by 16",
            shape[1]
        );
        assert!(
            data.len() == shape.iter().fold(1, |acc, next| acc * next),
            "Data of Length {} doesn't work for shape {:#?}",
            data.len(),
            shape
        );

        Self { shape, data }
    }
}

#[derive(Copy)]
struct F32Buffer(*mut f32);

unsafe impl Sync for F32Buffer {}
unsafe impl Send for F32Buffer {}
impl Clone for F32Buffer {
    fn clone(&self) -> Self {
        F32Buffer(self.0)
    }
}

impl F32Buffer {
    #[inline(always)]
    unsafe fn set(self, i: usize, v: f32) {
        *self.0.add(i) = v
    }
}

pub fn sgemm(a: &F32Tensor, a_t: bool, b: &F32Tensor, b_t: bool, c: &mut Vec<f32>) {
    assert!(!a_t && !b_t, "Transposes are not supported yet");
    assert!(
        a.shape[1] == b.shape[0],
        "Tensor A Shape {:#?} is not compatible with Tensor B Shape {:#?}",
        a.shape,
        b.shape
    );
    assert!(
        a.shape[0] * b.shape[1] == c.len(),
        "Output buffer `c` has size {}, but should have {} * {}",
        c.len(),
        a.shape[0],
        b.shape[1]
    );

    let m = a.shape[0];
    let n = a.shape[1];
    let p = b.shape[1];

    for i in 0..m {
        for j in 0..p {
            for k in 0..n {
                c[i * p + j] += a.data[i * n + k] * b.data[k * p + j];
            }
        }
    }
}

pub fn sgemm_tiled(a: &F32Tensor, a_t: bool, b: &F32Tensor, b_t: bool, c: &mut Vec<f32>) {
    assert!(!a_t && !b_t, "Transposes are not supported yet");
    assert!(
        a.shape[1] == b.shape[0],
        "Tensor A Shape {:#?} is not compatible with Tensor B Shape {:#?}",
        a.shape,
        b.shape
    );
    assert!(
        a.shape[0] * b.shape[1] == c.len(),
        "Output buffer `c` has size {}, but should have {} * {}",
        c.len(),
        a.shape[0],
        b.shape[1]
    );

    let m = a.shape[0];
    let n = a.shape[1];
    let p = b.shape[1];

    let block_size = 16;

    for col_block in (0..p).step_by(block_size) {
        for row in 0..m {
            for tile in (0..n).step_by(block_size) {
                for tile_row in 0..block_size {
                    for el in 0..block_size {
                        c[row * p + col_block + el] = a.data[row * n + tile + tile_row]
                            * b.data[tile * p + tile_row * p + col_block + el];
                    }
                }
            }
        }
    }
}

pub fn sgemm_tiled_par(a: &F32Tensor, a_t: bool, b: &F32Tensor, b_t: bool, c: &mut Vec<f32>) {
    assert!(!a_t && !b_t, "Transposes are not supported yet");
    assert!(
        a.shape[1] == b.shape[0],
        "Tensor A Shape {:#?} is not compatible with Tensor B Shape {:#?}",
        a.shape,
        b.shape
    );
    assert!(
        a.shape[0] * b.shape[1] == c.len(),
        "Output buffer `c` has size {}, but should have {} * {}",
        c.len(),
        a.shape[0],
        b.shape[1]
    );

    let m = a.shape[0];
    let n = a.shape[1];
    let p = b.shape[1];

    let block_size = 16;

    let c_ptr = F32Buffer(c.as_mut_ptr());

    (0..p)
        .into_par_iter()
        .step_by(block_size)
        .for_each(|col_block| {
            for row in 0..m {
                for tile in (0..n).step_by(block_size) {
                    for tile_row in 0..block_size {
                        for el in 0..block_size {
                            unsafe {
                                c_ptr.set(
                                    row * p + col_block + el,
                                    a.data[row * n + tile + tile_row]
                                        * b.data[tile * p + tile_row * p + col_block + el],
                                );
                            }
                        }
                    }
                }
            }
        });
}

pub fn sgemm_tiled_simd(a: &F32Tensor, a_t: bool, b: &F32Tensor, b_t: bool, c: &mut Vec<f32>) {
    assert!(!a_t && !b_t, "Transposes are not supported yet");
    assert!(
        a.shape[1] == b.shape[0],
        "Tensor A Shape {:#?} is not compatible with Tensor B Shape {:#?}",
        a.shape,
        b.shape
    );
    assert!(
        a.shape[0] * b.shape[1] == c.len(),
        "Output buffer `c` has size {}, but should have {} * {}",
        c.len(),
        a.shape[0],
        b.shape[1]
    );

    let m = a.shape[0];
    let n = a.shape[1];
    let p = b.shape[1];

    let block_size = 16;

    if is_x86_feature_detected!("avx") {
        println!("Using avx instructions.");
        for col_block in (0..p).step_by(block_size) {
            for row in 0..m {
                for tile in (0..n).step_by(block_size) {
                    for tile_col in 0..block_size {
                        unsafe {
                            let b_vector_1 = _mm256_loadu_ps(
                                b.data.as_ptr().add(tile * p + tile_col * p + col_block),
                            );
                            let b_vector_2 = _mm256_loadu_ps(
                                b.data.as_ptr().add(tile * p + tile_col * p + col_block + 8),
                            );

                            let a_values = _mm256_broadcast_ss(&a.data[row * n + tile + tile_col]);

                            let res_1 = _mm256_dp_ps(a_values, b_vector_1, 0);
                            let res_2 = _mm256_dp_ps(a_values, b_vector_2, 0);

                            _mm256_storeu_ps(c.as_mut_ptr().add(row * p + col_block), res_1);
                            _mm256_storeu_ps(c.as_mut_ptr().add(row * p + col_block + 8), res_2);
                        }
                    }
                }
            }
        }
    } else {
        println!("Using Naive Implementation. This might take a while.");
        for col_block in (0..p).step_by(block_size) {
            for row in 0..m {
                for tile in (0..n).step_by(block_size) {
                    for tile_row in 0..block_size {
                        for el in 0..block_size {
                            c[row * p + col_block + el] = a.data[row * n + tile + tile_row]
                                * b.data[tile * p + tile_row * p + col_block + el];
                        }
                    }
                }
            }
        }
    }
}
