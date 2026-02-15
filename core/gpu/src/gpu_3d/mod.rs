// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
pub mod consts;
pub mod geometry;
pub mod render;
pub mod structs;

use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};
use std::collections::VecDeque;

use crate::gpu_3d::structs::{
    Disp3DCntReg, Gpu3D, GxStatReg, Matrix, Polygon, PolygonAttrReg, TexImageParamReg, Vertex,
    ViewportReg,
};

impl Gpu3D {
    pub fn new() -> Self {
        Self {
            cycles: 100,
            disp3dcnt: Disp3DCntReg::default(),
            polygon_attr: PolygonAttrReg::default(),
            teximage_param: TexImageParamReg::default(),
            toon_table: [0; 32],
            pltt_base: 0,
            viewport: ViewportReg::default(),
            gxstat: GxStatReg::default(),
            polygon_type: 0,
            clear_depth: 0,
            clear_color: 0,
            flush_mode: 0,
            gxfifo: VecDeque::new(),
            gxpipe: VecDeque::new(),
            cmd_params: vec![0; 32],
            param_count: 0,
            cmd_param_count: 0,
            cmd_count: 0,
            total_params: 0,
            current_cmd: 0,
            current_poly_attr: PolygonAttrReg::default(),
            current_color: 0,
            current_vertex: [0, 0, 0],
            current_texcoords: [0, 0],
            z_buffer: vec![vec![0; PIXELS_PER_LINE]; SCANLINES],
            trans_poly_ids: vec![0; PIXELS_PER_LINE],
            swap_buffers: false,
            geo_vert: vec![Vertex::default(); 6188],
            rend_vert: vec![Vertex::default(); 6188],
            geo_poly: vec![Polygon::default(); 2048],
            rend_poly: vec![Polygon::default(); 2048],
            last_poly_strip: None,
            vertex_list: vec![Vertex::default(); 10],
            vertex_list_count: 0,
            geo_vert_count: 0,
            rend_vert_count: 0,
            geo_poly_count: 0,
            rend_poly_count: 0,
            consecutive_polygons: 0,
            vtx_16_index: 0,
            mtx_mode: 0,
            projection_mtx: Matrix::default(),
            vector_mtx: Matrix::default(),
            modelview_mtx: Matrix::default(),
            texture_mtx: Matrix::default(),
            projection_stack: Matrix::default(),
            texture_stack: Matrix::default(),
            modelview_stack: vec![Matrix::default(); 0x20],
            vector_stack: vec![Matrix::default(); 0x20],
            clip_mtx: Matrix::default(),
            clip_dirty: false,
            model_view_sp: 0,
            emission_color: 0,
            ambient_color: 0,
            diffuse_color: 0,
            specular_color: 0,
            light_color: [0; 4],
            light_direction: [[0; 3]; 4],
            normal_vector: [0; 3],
            shine_table: vec![0; 128],
            using_shine_table: false,
            vec_test_result: [0; 3],
            mult_params: Matrix::default(),
            mult_params_index: 0,
        }
    }
}
