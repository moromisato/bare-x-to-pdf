#include <assert.h>
#include <bare.h>
#include <js.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <utf.h>

#include "simple_converter.h"

static bool
simple_converter__string(js_env_t *env, js_value_t *value, char **result) {
  int err;

  size_t len = 0;
  err = js_get_value_string_utf8(env, value, NULL, 0, &len);
  if (err != 0) return false;

  char *str = malloc(len + 1);
  if (str == NULL) return false;

  err = js_get_value_string_utf8(env, value, (utf8_t *) str, len + 1, &len);
  if (err != 0) {
    free(str);
    return false;
  }

  str[len] = '\0';
  *result = str;
  return true;
}

static js_value_t *
simple_converter_convert(js_env_t *env, js_callback_info_t *info) {
  int err;

  size_t argc = 5;
  js_value_t *argv[5];

  err = js_get_callback_info(env, info, &argc, argv, NULL, NULL);
  assert(err == 0);

  if (argc < 5) {
    err = js_throw_error(env, NULL, "convert expects (bytes, from, to, fontsDir, pdfiumPath)");
    assert(err == 0);
    return NULL;
  }

  uint8_t *input;
  size_t input_len;
  err = js_get_typedarray_info(env, argv[0], NULL, (void **) &input, &input_len, NULL, NULL);
  if (err != 0) {
    err = js_throw_error(env, NULL, "input must be a Uint8Array");
    assert(err == 0);
    return NULL;
  }

  char *from = NULL;
  char *to = NULL;
  char *fonts_dir = NULL;
  char *pdfium_path = NULL;

  bool ok = simple_converter__string(env, argv[1], &from) &&
            simple_converter__string(env, argv[2], &to) &&
            simple_converter__string(env, argv[3], &fonts_dir) &&
            simple_converter__string(env, argv[4], &pdfium_path);

  if (!ok) {
    free(from);
    free(to);
    free(fonts_dir);
    free(pdfium_path);
    err = js_throw_error(env, NULL, "from, to, fontsDir and pdfiumPath must be strings");
    assert(err == 0);
    return NULL;
  }

  sc_buffer_t out = {NULL, 0};
  sc_buffer_t error = {NULL, 0};

  int status = sc_convert(input, input_len, from, to, fonts_dir, pdfium_path, &out, &error);

  free(from);
  free(to);
  free(fonts_dir);
  free(pdfium_path);

  if (status != 0) {
    char *message = malloc(error.len + 1);
    if (message != NULL) {
      memcpy(message, error.data, error.len);
      message[error.len] = '\0';
    }
    sc_free(&error);
    err = js_throw_error(env, NULL, message ? message : "conversion failed");
    assert(err == 0);
    free(message);
    return NULL;
  }

  js_value_t *result;
  void *data;
  err = js_create_arraybuffer(env, out.len, &data, &result);
  assert(err == 0);

  if (out.len > 0) memcpy(data, out.data, out.len);
  sc_free(&out);

  return result;
}

static js_value_t *
simple_converter_exports(js_env_t *env, js_value_t *exports) {
  int err;

#define V(name, fn) \
  { \
    js_value_t *val; \
    err = js_create_function(env, name, -1, fn, NULL, &val); \
    assert(err == 0); \
    err = js_set_named_property(env, exports, name, val); \
    assert(err == 0); \
  }

  V("convert", simple_converter_convert)
#undef V

  return exports;
}

BARE_MODULE(simple_converter, simple_converter_exports)
