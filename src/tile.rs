
use std::mem;

use cgmath::*;
use image::{Rgba, ImageBuffer};
use genmesh::Triangle;

use {Barycentric, Interpolate, Fragment, Mapping};
use f32x8::{f32x8x8, f32x8x8_vec3};


#[derive(Clone, Copy, Debug)]
pub struct TileMask {
    u: f32x8x8,
    v: f32x8x8,
    mask: u64
}

impl TileMask {
    #[inline(always)]
    /// Calculate the u/v coordinates for the fragment
    pub fn new(pos: Vector2<f32>, scale: Vector2<f32>, bary: &Barycentric) -> TileMask {
        let [u, v] =  bary.coordinate_f32x8x8(pos, scale);
        let uv = f32x8x8::broadcast(1.) - (u + v);

        let mask = !(uv.to_bit_u32x8x8().bitmask() |
                      u.to_bit_u32x8x8().bitmask() |
                      v.to_bit_u32x8x8().bitmask());

        TileMask {
            u: u,
            v: v,
            mask: mask
        }
    }

    #[inline(always)]
    pub fn mask_with_depth(&mut self, z: &Vector3<f32>, d: &mut f32x8x8) {
        let z = f32x8x8_vec3::broadcast(Vector3::new(z.x, z.y, z.z));
        let uv = f32x8x8::broadcast(1.) - (self.u + self.v);
        let weights = f32x8x8_vec3([uv, self.u, self.v]);
        let depth = weights.dot(z);

        self.mask &= (depth - *d).to_bit_u32x8x8().bitmask();
        self.mask &= !(f32x8x8::broadcast(1.) + depth).to_bit_u32x8x8().bitmask();
        d.replace(depth, self.mask);
    }

    #[inline]
    pub fn iter(self) -> TileMaskIter {
        TileMaskIter {
            u: unsafe { mem::transmute(self.u) },
            v: unsafe { mem::transmute(self.v) },
            mask: self.mask
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TileIndex(pub u32);

impl TileIndex {
    #[inline]
    pub fn from_xy(x: u32, y: u32) -> TileIndex {
        TileIndex(x*8+y)
    }
    #[inline] pub fn x(self) -> u32 { (self.0 as u32)  & 0x7 }
    #[inline] pub fn y(self) -> u32 { (self.0 as u32)  >> 3 }
    #[inline] pub fn x8(self) -> u32 { self.x() * 8 }
    #[inline] pub fn y8(self) -> u32 { self.y() * 8 }
}

pub struct TileMaskIter {
    u: [f32; 64],
    v: [f32; 64],
    mask: u64
}

impl Iterator for TileMaskIter {
    type Item = (TileIndex, [f32; 3]);

    #[inline]
    fn next(&mut self) -> Option<(TileIndex, [f32; 3])> {
        if self.mask == 0 {
            return None;
        }

        let next = self.mask.trailing_zeros();
        self.mask &= !(1 << next);

        unsafe {
            let u = self.u.get_unchecked(next as usize);
            let v = self.v.get_unchecked(next as usize);
            Some((
                TileIndex(next as u32),
                [1. - (u + v), *u, *v]

            ))
        }
    }
}

#[derive(Copy)]
pub struct Tile<P> {
    depth: f32x8x8,
    color: [P; 64],
}

impl<P: Copy> Clone for Tile<P> {
    fn clone(&self) -> Tile<P> {
        Tile {
            depth: self.depth,
            color: self.color
        }
    }
}

impl<P: Copy> Tile<P> {
    pub fn new(p: P) -> Tile<P> {
         Tile {
            depth: f32x8x8::broadcast(1.),
            color: [p; 64]
        }       
    }
}

#[derive(Copy)]
struct Quad<T>(pub [T; 4]);

impl<T: Copy> Quad<T> {
    pub fn new(t: T) -> Quad<T> {
        Quad([t, t, t, t])
    }
}

impl<T: Copy> Clone for Quad<T> {
    fn clone(&self) -> Quad<T> {
        Quad(self.0)
    }
}

#[derive(Copy)]
pub struct TileGroup<P> {
    tiles: Quad<Quad<Tile<P>>>
}

impl<P: Copy> Clone for TileGroup<P> {
    fn clone(&self) -> TileGroup<P> {
        TileGroup {
            tiles: self.tiles
        }
    }
}

impl<P: Copy> TileGroup<P> {
    pub fn new(p: P) -> TileGroup<P> {
        TileGroup {
            tiles: Quad::new(Quad::new(Tile::new(p)))
        }
    }

    pub fn write<W: Put<P>>(&self, x: u32, y: u32, v: &mut W) {
        self.tiles.write(x, y, v);
    }

    pub fn raster<F, T, O>(&mut self,
                           pos: Vector2<f32>,
                           scale: Vector2<f32>,
                           z: &Vector3<f32>,
                           bary: &Barycentric,
                           t: &Triangle<T>,
                           fragment: &F) where
              T: Interpolate<Out=O>,
              F: Fragment<O, Color=P> {

        self.tiles.raster(pos, scale, z, bary, t, fragment);
    }

    pub fn clear(&mut self, p: P) {
        Raster::clear(&mut self.tiles, p);
    }

    pub fn map<S, F>(&mut self, src: &TileGroup<S>, f: &F) where F: Mapping<S, Out=P>, S: Copy {
        self.tiles.map(&src.tiles, f);
    }
}

pub trait Raster<P> {
    fn mask(&self) -> u32 { 0xFFFF_FFFF - (self.size() - 1) }
    fn size(&self) -> u32;
    fn raster<F, T, O>(&mut self,
                       pos: Vector2<f32>,
                       scale: Vector2<f32>,
                       z: &Vector3<f32>,
                       bary: &Barycentric,
                       t: &Triangle<T>,
                       fragment: &F) where
              T: Interpolate<Out=O>,
              F: Fragment<O, Color=P>;

    fn clear(&mut self, p: P);
    fn write<W: Put<P>>(&self, x: u32, y: u32, v: &mut W);
}

pub trait ApplyMapping<P, T, P2> {
    fn map<F>(&mut self, src: &T, f: &F) where F: Mapping<P2, Out=P>;
}

impl<I, P: Copy> Raster<P> for Quad<I> where I: Raster<P> {
    #[inline]
    fn size(&self) -> u32 { 2 * self.0[0].size() }

    #[inline]
    fn raster<F, T, O>(&mut self,
                       pos: Vector2<f32>,
                       scale: Vector2<f32>,
                       z: &Vector3<f32>,
                       bary: &Barycentric,
                       t: &Triangle<T>,
                       fragment: &F) where
              T: Interpolate<Out=O>,
              F: Fragment<O, Color=P> {

        let tsize = scale.mul_s(self.0[0].size() as f32);
        self.0[0].raster(pos,                     scale, z, bary, t, fragment);
        self.0[1].raster(pos + vec2(tsize.x, 0.), scale, z, bary, t, fragment);
        self.0[2].raster(pos + vec2(0., tsize.y), scale, z, bary, t, fragment);
        self.0[3].raster(pos + tsize,             scale, z, bary, t, fragment);
    }

    #[inline]
    fn clear(&mut self, p: P) {
        for i in self.0.iter_mut() {
            i.clear(p)
        }
    }

    #[inline]
    fn write<W: Put<P>>(&self, x: u32, y: u32, v: &mut W) {
        let tsize = self.0[0].size();
        self.0[0].write(x,       y,       v);
        self.0[1].write(x+tsize, y,       v);
        self.0[2].write(x,       y+tsize, v);
        self.0[3].write(x+tsize, y+tsize, v);
    }
}

impl<I, I2, P, P2> ApplyMapping<P, Quad<I2>, P2> for Quad<I> where I: ApplyMapping<P, I2, P2> {
    fn map<F>(&mut self, src: &Quad<I2>, f: &F) where F: Mapping<P2, Out=P> {
        for (dst, src) in self.0.iter_mut().zip(src.0.iter()) {
            dst.map(src, f);
        }
    }
}

impl<P: Copy> Raster<P> for Tile<P> {
    #[inline]
    fn size(&self) -> u32 { 8 }

    #[inline]
    fn raster<F, T, O>(&mut self,
                       pos: Vector2<f32>,
                       scale: Vector2<f32>,
                       z: &Vector3<f32>,
                       bary: &Barycentric,
                       t: &Triangle<T>,
                       fragment: &F) where
              T: Interpolate<Out=O>,
              F: Fragment<O, Color=P> {

        let mut mask = TileMask::new(pos, scale, &bary);
        if mask.mask == 0 {
            return;
        }

        mask.mask_with_depth(z, &mut self.depth);
        for (i, w) in mask.iter() {
            let frag = Interpolate::interpolate(t, w);
            let new = fragment.fragment(frag);
            let dst = unsafe { self.color.get_unchecked_mut(i.0 as usize) };
            *dst = fragment.blend(*dst, new);
        }
    }

    #[inline]
    fn write<W: Put<P>>(&self, x: u32, y: u32, v: &mut W) {
        for i in (0..64).map(|x| TileIndex(x)) {
            v.put(x+i.x(), y+i.y(), self.color[i.0 as usize]);
        }
    }

    #[inline]
    fn clear(&mut self, p: P) {
        self.depth = f32x8x8::broadcast(1.);
        self.color = [p; 64];
    }
}

impl<T: Copy, P> ApplyMapping<P, Tile<T>, T> for Tile<P> {
    fn map<F>(&mut self, src: &Tile<T>, f: &F) where F: Mapping<T, Out=P> {
        for (dst, src) in self.color.iter_mut().zip(src.color.iter()) {
            *dst = f.mapping(*src);
        }
    }
}

pub trait Put<P> {
    fn put(&mut self, x: u32, y: u32, v: P);
}

impl Put<Rgba<u8>> for ImageBuffer<Rgba<u8>, Vec<u8>> {
    fn put(&mut self, x: u32, y: u32, p: Rgba<u8>) {
        let h = self.height();
        self.put_pixel(x, h - 1 - y, p);
    }
}
