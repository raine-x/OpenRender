/* Minimal C host for the OpenRender C ABI.
 *
 * Build (desktop, dynamic):
 *   cargo build -p openrender-core --release
 *   cc examples/c/demo.c -Iinclude -Ltarget/release -lopenrender -o demo
 *   ./demo                     (Windows: -Ltarget/release -lopenrender.dll / .lib)
 *
 * Build (Android, static): see scripts/build-android.sh and
 * android/jniLibs/static/<abi>/libopenrender.a
 */

#include <stdio.h>
#include <string.h>

#include "engine.h"

int main(void) {
  OrEngine *engine = or_engine_new(NULL);
  if (engine == NULL) {
    fprintf(stderr, "failed to create engine\n");
    return 1;
  }

  const char *chunks[] = {
      "# Hello from C\n\n",
      "This is **bold so far",
      "** and finished.\n\n",
      "```rust\nfn main() {}\n```\n",
  };

  for (size_t i = 0; i < sizeof(chunks) / sizeof(chunks[0]); ++i) {
    char *delta = or_engine_feed(engine, chunks[i], strlen(chunks[i]));
    if (delta != NULL) {
      printf("feed[%zu] %s\n", i, delta);
      or_string_free(delta);
    }
    /* default 8 ms throttle: drain any buffered text before printing */
    char *flush = or_engine_flush(engine);
    if (flush != NULL) {
      printf("flush[%zu] %s\n", i, flush);
      or_string_free(flush);
    }
  }

  char *done = or_engine_finish(engine);
  if (done != NULL) {
    printf("finish   %s\n", done);
    or_string_free(done);
  }

  or_engine_free(engine);
  return 0;
}
