// Rust へ写した構造体の並びを、同梱ヘッダの実物と突き合わせるための出口。
// 並べる順は lib.rs の突き合わせと対応する。

#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include <cstddef>

#include <aviutl2_sdk/logger2.h>
#include <aviutl2_sdk/output2.h>

extern "C" {

size_t aviutl2_sys_sizeof_output_info(void) { return sizeof(OUTPUT_INFO); }

size_t aviutl2_sys_sizeof_output_plugin_table(void) {
  return sizeof(OUTPUT_PLUGIN_TABLE);
}

size_t aviutl2_sys_sizeof_log_handle(void) { return sizeof(LOG_HANDLE); }

void aviutl2_sys_output_info_offsets(size_t out[15]) {
  size_t* p = out;
  *p++ = offsetof(OUTPUT_INFO, flag);
  *p++ = offsetof(OUTPUT_INFO, w);
  *p++ = offsetof(OUTPUT_INFO, h);
  *p++ = offsetof(OUTPUT_INFO, rate);
  *p++ = offsetof(OUTPUT_INFO, scale);
  *p++ = offsetof(OUTPUT_INFO, n);
  *p++ = offsetof(OUTPUT_INFO, audio_rate);
  *p++ = offsetof(OUTPUT_INFO, audio_ch);
  *p++ = offsetof(OUTPUT_INFO, audio_n);
  *p++ = offsetof(OUTPUT_INFO, savefile);
  *p++ = offsetof(OUTPUT_INFO, func_get_video);
  *p++ = offsetof(OUTPUT_INFO, func_get_audio);
  *p++ = offsetof(OUTPUT_INFO, func_is_abort);
  *p++ = offsetof(OUTPUT_INFO, func_rest_time_disp);
  *p++ = offsetof(OUTPUT_INFO, func_set_buffer_size);
}

void aviutl2_sys_output_plugin_table_offsets(size_t out[9]) {
  size_t* p = out;
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, flag);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, name);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, filefilter);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, information);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, func_output);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, func_config);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, func_get_config_text);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, func_load_project_config);
  *p++ = offsetof(OUTPUT_PLUGIN_TABLE, func_save_project_config);
}

void aviutl2_sys_log_handle_offsets(size_t out[5]) {
  size_t* p = out;
  *p++ = offsetof(LOG_HANDLE, log);
  *p++ = offsetof(LOG_HANDLE, info);
  *p++ = offsetof(LOG_HANDLE, warn);
  *p++ = offsetof(LOG_HANDLE, error);
  *p++ = offsetof(LOG_HANDLE, verbose);
}

void aviutl2_sys_output_info_flags(int out[2]) {
  int* p = out;
  *p++ = OUTPUT_INFO::FLAG_VIDEO;
  *p++ = OUTPUT_INFO::FLAG_AUDIO;
}

void aviutl2_sys_output_plugin_table_flags(int out[4]) {
  int* p = out;
  *p++ = OUTPUT_PLUGIN_TABLE::FLAG_VIDEO;
  *p++ = OUTPUT_PLUGIN_TABLE::FLAG_AUDIO;
  *p++ = OUTPUT_PLUGIN_TABLE::FLAG_IMAGE;
  *p++ = OUTPUT_PLUGIN_TABLE::FLAG_PROJECT_CONFIG;
}
}
