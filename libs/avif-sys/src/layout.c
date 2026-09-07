// Rust へ写した構造体の並びを、同梱ヘッダの実物と突き合わせるための出口。
// 並べる順は lib.rs の突き合わせと対応する。

#include <stddef.h>

#include "avif/avif.h"

size_t avif_sys_sizeof_encoder(void) { return sizeof(avifEncoder); }

size_t avif_sys_sizeof_image(void) { return sizeof(avifImage); }

size_t avif_sys_sizeof_rgb_image(void) { return sizeof(avifRGBImage); }

size_t avif_sys_sizeof_rw_data(void) { return sizeof(avifRWData); }

size_t avif_sys_sizeof_diagnostics(void) { return sizeof(avifDiagnostics); }

void avif_sys_encoder_offsets(size_t out[8]) {
  size_t* p = out;
  *p++ = offsetof(avifEncoder, maxThreads);
  *p++ = offsetof(avifEncoder, speed);
  *p++ = offsetof(avifEncoder, timescale);
  *p++ = offsetof(avifEncoder, repetitionCount);
  *p++ = offsetof(avifEncoder, quality);
  *p++ = offsetof(avifEncoder, qualityAlpha);
  *p++ = offsetof(avifEncoder, ioStats);
  *p++ = offsetof(avifEncoder, diag);
}

void avif_sys_image_offsets(size_t out[9]) {
  size_t* p = out;
  *p++ = offsetof(avifImage, width);
  *p++ = offsetof(avifImage, height);
  *p++ = offsetof(avifImage, depth);
  *p++ = offsetof(avifImage, yuvFormat);
  *p++ = offsetof(avifImage, yuvRange);
  *p++ = offsetof(avifImage, alphaPlane);
  *p++ = offsetof(avifImage, colorPrimaries);
  *p++ = offsetof(avifImage, transferCharacteristics);
  *p++ = offsetof(avifImage, matrixCoefficients);
}

void avif_sys_rgb_image_offsets(size_t out[13]) {
  size_t* p = out;
  *p++ = offsetof(avifRGBImage, width);
  *p++ = offsetof(avifRGBImage, height);
  *p++ = offsetof(avifRGBImage, depth);
  *p++ = offsetof(avifRGBImage, format);
  *p++ = offsetof(avifRGBImage, chromaUpsampling);
  *p++ = offsetof(avifRGBImage, chromaDownsampling);
  *p++ = offsetof(avifRGBImage, avoidLibYUV);
  *p++ = offsetof(avifRGBImage, ignoreAlpha);
  *p++ = offsetof(avifRGBImage, alphaPremultiplied);
  *p++ = offsetof(avifRGBImage, isFloat);
  *p++ = offsetof(avifRGBImage, maxThreads);
  *p++ = offsetof(avifRGBImage, pixels);
  *p++ = offsetof(avifRGBImage, rowBytes);
}

void avif_sys_rw_data_offsets(size_t out[2]) {
  size_t* p = out;
  *p++ = offsetof(avifRWData, data);
  *p++ = offsetof(avifRWData, size);
}

void avif_sys_io_stats_offsets(size_t out[2]) {
  size_t* p = out;
  *p++ = offsetof(avifIOStats, colorOBUSize);
  *p++ = offsetof(avifIOStats, alphaOBUSize);
}
