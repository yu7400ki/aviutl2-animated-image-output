// Rust へ写した構造体の並びを、同梱ヘッダの実物と突き合わせるための出口。
// 並べる順は lib.rs の突き合わせと対応する。

#include <stddef.h>

#include "src/webp/encode.h"

size_t webp_sys_sizeof_config(void) { return sizeof(WebPConfig); }

size_t webp_sys_sizeof_picture(void) { return sizeof(WebPPicture); }

size_t webp_sys_sizeof_memory_writer(void) { return sizeof(WebPMemoryWriter); }

void webp_sys_config_offsets(size_t out[29]) {
  size_t* p = out;
  *p++ = offsetof(WebPConfig, lossless);
  *p++ = offsetof(WebPConfig, quality);
  *p++ = offsetof(WebPConfig, method);
  *p++ = offsetof(WebPConfig, image_hint);
  *p++ = offsetof(WebPConfig, target_size);
  *p++ = offsetof(WebPConfig, target_PSNR);
  *p++ = offsetof(WebPConfig, segments);
  *p++ = offsetof(WebPConfig, sns_strength);
  *p++ = offsetof(WebPConfig, filter_strength);
  *p++ = offsetof(WebPConfig, filter_sharpness);
  *p++ = offsetof(WebPConfig, filter_type);
  *p++ = offsetof(WebPConfig, autofilter);
  *p++ = offsetof(WebPConfig, alpha_compression);
  *p++ = offsetof(WebPConfig, alpha_filtering);
  *p++ = offsetof(WebPConfig, alpha_quality);
  *p++ = offsetof(WebPConfig, pass);
  *p++ = offsetof(WebPConfig, show_compressed);
  *p++ = offsetof(WebPConfig, preprocessing);
  *p++ = offsetof(WebPConfig, partitions);
  *p++ = offsetof(WebPConfig, partition_limit);
  *p++ = offsetof(WebPConfig, emulate_jpeg_size);
  *p++ = offsetof(WebPConfig, thread_level);
  *p++ = offsetof(WebPConfig, low_memory);
  *p++ = offsetof(WebPConfig, near_lossless);
  *p++ = offsetof(WebPConfig, exact);
  *p++ = offsetof(WebPConfig, use_delta_palette);
  *p++ = offsetof(WebPConfig, use_sharp_yuv);
  *p++ = offsetof(WebPConfig, qmin);
  *p++ = offsetof(WebPConfig, qmax);
}

void webp_sys_picture_offsets(size_t out[21]) {
  size_t* p = out;
  *p++ = offsetof(WebPPicture, use_argb);
  *p++ = offsetof(WebPPicture, colorspace);
  *p++ = offsetof(WebPPicture, width);
  *p++ = offsetof(WebPPicture, height);
  *p++ = offsetof(WebPPicture, y);
  *p++ = offsetof(WebPPicture, u);
  *p++ = offsetof(WebPPicture, v);
  *p++ = offsetof(WebPPicture, y_stride);
  *p++ = offsetof(WebPPicture, uv_stride);
  *p++ = offsetof(WebPPicture, a);
  *p++ = offsetof(WebPPicture, a_stride);
  *p++ = offsetof(WebPPicture, argb);
  *p++ = offsetof(WebPPicture, argb_stride);
  *p++ = offsetof(WebPPicture, writer);
  *p++ = offsetof(WebPPicture, custom_ptr);
  *p++ = offsetof(WebPPicture, extra_info_type);
  *p++ = offsetof(WebPPicture, extra_info);
  *p++ = offsetof(WebPPicture, stats);
  *p++ = offsetof(WebPPicture, error_code);
  *p++ = offsetof(WebPPicture, progress_hook);
  *p++ = offsetof(WebPPicture, user_data);
}

void webp_sys_memory_writer_offsets(size_t out[3]) {
  size_t* p = out;
  *p++ = offsetof(WebPMemoryWriter, mem);
  *p++ = offsetof(WebPMemoryWriter, size);
  *p++ = offsetof(WebPMemoryWriter, max_size);
}
