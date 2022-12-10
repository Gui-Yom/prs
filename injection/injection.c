#include <netinet/in.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Functions implemented by the rust library
extern void inject_start();
extern void trace_server_ip(int);
extern void push_msg(int);
extern void push_drop(int);
extern void push_ack(int);
extern void end_total(int);

// Overriding fork so we can fflush before.
pid_t fork(void) {
  // Fix a client bug with printf buffers not being flushed before fork()
  fflush(stdout);
  return __fork();
}

// Does `string` starts with `prefix` ?
_Bool starts_with(const char *restrict string, const char *restrict prefix) {
  while (*prefix) {
    if (*prefix++ != *string++)
      return 0;
  }
  return 1;
}

// Also hooking puts, so we can shut down all stdout output and also detect the
// moment the program starts.
int puts(const char *s) {
  if (starts_with(s, "Sending SYN")) {
    inject_start();
  }
  return 0;
}

// Log processing, the rust side does all the heavy lifting while this C code
// simply dispatch messages to the corresponding functions.
int printf(const char *format, ...) {
  va_list args;
  va_start(args, format);

  if (starts_with(format, "Message received:")) {
    // First arg is size
    va_arg(args, int);
    // Then 6 char for the sequence number
    char seq[7] = "000000";
    for (int i = 0; i < 6; ++i) {
      seq[i] = (char)va_arg(args, int);
    }
    push_msg(atoi(seq));
  } else if (starts_with(format, "Message ")) {
    // Sequence number
    push_drop(va_arg(args, int));
  } else if (starts_with(format, "Sent ")) {
    char *ack = va_arg(args, char *);
    push_ack(atoi(ack + 3));
  } else if (starts_with(format, "Total bytes ")) {
    end_total(va_arg(args, int));
  } else if (starts_with(format, "Asking for")) {
    // Accessing a variable from the caller function
    int offset = 0x2511c;      // Client 2
    if (strlen(format) > 36) { // Client 1
      offset = 0x256bc;
    }
    // Stack  frame base pointer
    register long rbp asm("rbp");
    // Go to the stack frame of the caller function (client main) then offset to
    // the ip location
    trace_server_ip(ntohl(*(int *)(*(long int *)rbp - offset)));
  }

  va_end(args);
  return 0;
}
