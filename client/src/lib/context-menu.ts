// One gesture contract for plate, queue and history rows.
export function contextMenu(
  row: HTMLElement,
  open: (row: HTMLElement) => void,
) {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let x = 0,
    y = 0,
    pressed = false;
  const cancel = () => clearTimeout(timer);
  const show = (event: Event) => {
    event.preventDefault();
    cancel();
    open(row);
  };
  const options = { signal: controller.signal };
  row.addEventListener("contextmenu", show, options);
  row.addEventListener(
    "keydown",
    (event) => {
      if (
        event.key === "ContextMenu" ||
        (event.shiftKey && event.key === "F10")
      )
        show(event);
    },
    options,
  );
  row.addEventListener(
    "pointerdown",
    (event) => {
      cancel();
      pressed = false;
      if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
      x = event.clientX;
      y = event.clientY;
      timer = setTimeout(() => {
        pressed = true;
        if (event.pointerType === "touch")
          row.addEventListener("touchend", (e) => e.preventDefault(), {
            ...options,
            once: true,
            passive: false,
          });
        open(row);
      }, 500);
    },
    options,
  );
  row.addEventListener(
    "pointermove",
    (event) => {
      if (Math.hypot(event.clientX - x, event.clientY - y) > 10) cancel();
    },
    options,
  );
  for (const name of ["pointerup", "pointercancel", "pointerleave"])
    row.addEventListener(name, cancel, options);
  row.addEventListener(
    "click",
    (event) => {
      if (pressed) {
        event.preventDefault();
        event.stopImmediatePropagation();
        pressed = false;
      }
    },
    { ...options, capture: true },
  );
  return {
    destroy() {
      cancel();
      controller.abort();
    },
  };
}
