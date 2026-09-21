<script lang="ts">
  import StlPreview from "./StlPreview.svelte";
  import { previewPosition } from "./preview-position";
  let { root }: { root?: HTMLElement } = $props();
  let active = $state<{
    url: string;
    name: string;
    left: number;
    top: number;
    width: number;
    height: number;
  }>();
  $effect(() => {
    if (!root) return;
    const container = root;
    let touch = false;
    let gone = false;
    const close = () => {
      active = undefined;
    };
    const show = (node: Element | null) => {
      if (!(node instanceof HTMLElement) || !container.contains(node))
        return close();
      const url = node.dataset.stlPreview;
      if (!url) return close();
      const controls = [
        ...container.querySelectorAll("input, button, select, textarea"),
        ...document.querySelectorAll("header"),
      ]
        .map((e) => e.getBoundingClientRect())
        .filter(
          (r) =>
            r.width &&
            r.height &&
            r.bottom > 0 &&
            r.top < innerHeight &&
            r.right > 0 &&
            r.left < innerWidth,
        );
      const position = previewPosition(
        node.getBoundingClientRect(),
        innerWidth,
        innerHeight,
        controls,
      );
      active = position
        ? { ...position, url, name: node.textContent ?? "" }
        : undefined;
    };
    const hover = (event: PointerEvent) => {
      if (event.pointerType === "mouse")
        show((event.target as Element).closest("[data-stl-preview]"));
    };
    const leave = (event: PointerEvent) => {
      const node = (event.target as Element).closest("[data-stl-preview]");
      if (node && !node.contains(event.relatedTarget as Node | null)) close();
    };
    const blur = () =>
      queueMicrotask(() => {
        if (!gone && !container.contains(document.activeElement)) close();
      });
    const focus = (event: FocusEvent) => {
      if (!touch)
        show(
          (event.target as Element)
            .closest("li")
            ?.querySelector("[data-stl-preview]") ?? null,
        );
    };
    const key = (event: KeyboardEvent) => {
      touch = false;
      if (event.key === "Escape") close();
    };
    const pointer = (event: PointerEvent) => {
      touch = event.pointerType === "touch";
      close();
    };
    container.addEventListener("pointerover", hover);
    container.addEventListener("pointerout", leave);
    container.addEventListener("focusin", focus);
    container.addEventListener("focusout", blur);
    container.addEventListener("input", close);
    window.addEventListener("pointerdown", pointer, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      gone = true;
      container.removeEventListener("pointerover", hover);
      container.removeEventListener("pointerout", leave);
      container.removeEventListener("focusin", focus);
      container.removeEventListener("focusout", blur);
      container.removeEventListener("input", close);
      window.removeEventListener("pointerdown", pointer, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  });
</script>

{#if active}
  <div
    class="hover-preview"
    role="tooltip"
    aria-label={`${active.name}の形状`}
    style:left={`${active.left}px`}
    style:top={`${active.top}px`}
    style:width={`${active.width}px`}
    style:height={`${active.height}px`}
  >
    <StlPreview url={active.url} name={active.name} passive />
  </div>
{/if}

<style lang="sass">
  .hover-preview
    position: fixed
    z-index: 6
    pointer-events: none
    padding: var(--sp-2)
    background: var(--c-surface-raised)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    box-shadow: 0 4px 16px var(--c-scrim)
</style>
