#ifndef SIMPLE_CONVERTER_H
#define SIMPLE_CONVERTER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
  uint8_t *data;
  size_t len;
} sc_buffer_t;

int
sc_convert(
  const uint8_t *input,
  size_t input_len,
  const char *from,
  const char *to,
  const char *fonts_dir,
  const char *pdfium_path,
  sc_buffer_t *out,
  sc_buffer_t *error
);

void
sc_free(sc_buffer_t *buffer);

#ifdef __cplusplus
}
#endif

#endif
