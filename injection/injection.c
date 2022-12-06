#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

extern void inject_start();
extern void trace_server_ip(const char *);
extern void push_msg(int);
extern void push_drop(int);
extern void push_ack(int);
extern void end_total(int);

pid_t fork(void) {
  // Fix a client bug with printf buffers not being flushed before fork()
  fflush(stdout);
  return __fork();
}

_Bool starts_with(const char *restrict string, const char *restrict prefix) {
  while (*prefix) {
    if (*prefix++ != *string++)
      return 0;
  }
  return 1;
}

int puts(const char *s) {
  if (starts_with(s, "Sending SYN")) {
    inject_start();
  }
  // Mute all calls to puts
  return 0;
}

int printf(const char *format, ...) {
  va_list args;
  va_start(args, format);

  // vfprintf(stdout, format, args);

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
    va_arg(args, char *);
    char *ip = va_arg(args, char *);
    //_IO_printf("ip: %s", ip);
    trace_server_ip(ip);
  }

  va_end(args);
  return 0;
}
