// Rust へ写した構造体の並びを、同梱ヘッダの実物と突き合わせるための出口。
// 並べる順は lib.rs の突き合わせと対応する。

#include <stddef.h>

#include "jxl/encode.h"
#include "jxl/thread_parallel_runner.h"

void jxl_sys_sizes(size_t out[8]) {
  size_t* p = out;
  *p++ = sizeof(JxlPixelFormat);
  *p++ = sizeof(JxlPreviewHeader);
  *p++ = sizeof(JxlAnimationHeader);
  *p++ = sizeof(JxlBasicInfo);
  *p++ = sizeof(JxlBlendInfo);
  *p++ = sizeof(JxlLayerInfo);
  *p++ = sizeof(JxlFrameHeader);
  *p++ = sizeof(JxlColorEncoding);
}

void jxl_sys_pixel_format_offsets(size_t out[4]) {
  size_t* p = out;
  *p++ = offsetof(JxlPixelFormat, num_channels);
  *p++ = offsetof(JxlPixelFormat, data_type);
  *p++ = offsetof(JxlPixelFormat, endianness);
  *p++ = offsetof(JxlPixelFormat, align);
}

void jxl_sys_preview_header_offsets(size_t out[2]) {
  size_t* p = out;
  *p++ = offsetof(JxlPreviewHeader, xsize);
  *p++ = offsetof(JxlPreviewHeader, ysize);
}

void jxl_sys_animation_header_offsets(size_t out[4]) {
  size_t* p = out;
  *p++ = offsetof(JxlAnimationHeader, tps_numerator);
  *p++ = offsetof(JxlAnimationHeader, tps_denominator);
  *p++ = offsetof(JxlAnimationHeader, num_loops);
  *p++ = offsetof(JxlAnimationHeader, have_timecodes);
}

void jxl_sys_basic_info_offsets(size_t out[23]) {
  size_t* p = out;
  *p++ = offsetof(JxlBasicInfo, have_container);
  *p++ = offsetof(JxlBasicInfo, xsize);
  *p++ = offsetof(JxlBasicInfo, ysize);
  *p++ = offsetof(JxlBasicInfo, bits_per_sample);
  *p++ = offsetof(JxlBasicInfo, exponent_bits_per_sample);
  *p++ = offsetof(JxlBasicInfo, intensity_target);
  *p++ = offsetof(JxlBasicInfo, min_nits);
  *p++ = offsetof(JxlBasicInfo, relative_to_max_display);
  *p++ = offsetof(JxlBasicInfo, linear_below);
  *p++ = offsetof(JxlBasicInfo, uses_original_profile);
  *p++ = offsetof(JxlBasicInfo, have_preview);
  *p++ = offsetof(JxlBasicInfo, have_animation);
  *p++ = offsetof(JxlBasicInfo, orientation);
  *p++ = offsetof(JxlBasicInfo, num_color_channels);
  *p++ = offsetof(JxlBasicInfo, num_extra_channels);
  *p++ = offsetof(JxlBasicInfo, alpha_bits);
  *p++ = offsetof(JxlBasicInfo, alpha_exponent_bits);
  *p++ = offsetof(JxlBasicInfo, alpha_premultiplied);
  *p++ = offsetof(JxlBasicInfo, preview);
  *p++ = offsetof(JxlBasicInfo, animation);
  *p++ = offsetof(JxlBasicInfo, intrinsic_xsize);
  *p++ = offsetof(JxlBasicInfo, intrinsic_ysize);
  *p++ = offsetof(JxlBasicInfo, padding);
}

void jxl_sys_blend_info_offsets(size_t out[4]) {
  size_t* p = out;
  *p++ = offsetof(JxlBlendInfo, blendmode);
  *p++ = offsetof(JxlBlendInfo, source);
  *p++ = offsetof(JxlBlendInfo, alpha);
  *p++ = offsetof(JxlBlendInfo, clamp);
}

void jxl_sys_layer_info_offsets(size_t out[7]) {
  size_t* p = out;
  *p++ = offsetof(JxlLayerInfo, have_crop);
  *p++ = offsetof(JxlLayerInfo, crop_x0);
  *p++ = offsetof(JxlLayerInfo, crop_y0);
  *p++ = offsetof(JxlLayerInfo, xsize);
  *p++ = offsetof(JxlLayerInfo, ysize);
  *p++ = offsetof(JxlLayerInfo, blend_info);
  *p++ = offsetof(JxlLayerInfo, save_as_reference);
}

void jxl_sys_frame_header_offsets(size_t out[5]) {
  size_t* p = out;
  *p++ = offsetof(JxlFrameHeader, duration);
  *p++ = offsetof(JxlFrameHeader, timecode);
  *p++ = offsetof(JxlFrameHeader, name_length);
  *p++ = offsetof(JxlFrameHeader, is_last);
  *p++ = offsetof(JxlFrameHeader, layer_info);
}

void jxl_sys_color_encoding_offsets(size_t out[10]) {
  size_t* p = out;
  *p++ = offsetof(JxlColorEncoding, color_space);
  *p++ = offsetof(JxlColorEncoding, white_point);
  *p++ = offsetof(JxlColorEncoding, white_point_xy);
  *p++ = offsetof(JxlColorEncoding, primaries);
  *p++ = offsetof(JxlColorEncoding, primaries_red_xy);
  *p++ = offsetof(JxlColorEncoding, primaries_green_xy);
  *p++ = offsetof(JxlColorEncoding, primaries_blue_xy);
  *p++ = offsetof(JxlColorEncoding, transfer_function);
  *p++ = offsetof(JxlColorEncoding, gamma);
  *p++ = offsetof(JxlColorEncoding, rendering_intent);
}
