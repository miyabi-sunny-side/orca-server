// Adapted from scad-live 88e840be (MIT); see THIRD_PARTY_NOTICES.
export class LatestRequest {
  #generation = 0;

  begin() {
    const generation = ++this.#generation;
    return Object.freeze({
      isCurrent: () => generation === this.#generation,
    });
  }

  invalidate() {
    this.#generation += 1;
  }
}
