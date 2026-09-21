// Adapted from scad-live 88e840be (MIT); see THIRD_PARTY_NOTICES.
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { STLLoader } from "three/addons/loaders/STLLoader.js";
import { LatestRequest } from "./latest-request";

export function createThreeViewer(mount: HTMLElement) {
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(38, 1, 0.01, 100000);
  camera.up.set(0, 0, 1);
  const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.domElement.tabIndex = 0;
  renderer.domElement.setAttribute("role", "application");
  renderer.domElement.setAttribute(
    "aria-label",
    "3Dモデル。ドラッグで回転、ホイールで拡大縮小。矢印キーで移動、Shiftと矢印キーで回転。",
  );
  mount.appendChild(renderer.domElement);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.screenSpacePanning = false;
  controls.listenToKeyEvents(renderer.domElement);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x777777, 2.4));
  const light = new THREE.DirectionalLight(0xffffff, 2.2);
  light.position.set(60, -70, 110);
  scene.add(light);
  const grid = new THREE.GridHelper(400, 40);
  grid.material.vertexColors = false;
  grid.rotation.x = Math.PI / 2;
  grid.position.z = -0.06;
  scene.add(grid);
  const material = new THREE.MeshStandardMaterial({
    roughness: 0.72,
    metalness: 0.04,
  });
  const theme = () => {
    const style = getComputedStyle(mount);
    material.color.set(style.getPropertyValue("--c-muted").trim());
    grid.material.color.set(style.getPropertyValue("--c-border").trim());
  };
  theme();
  const observer = new MutationObserver(theme);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  const scheme = matchMedia("(prefers-color-scheme: dark)");
  scheme.addEventListener("change", theme);
  const size = new THREE.Vector3();
  const fit = () => {
    const maximum = Math.max(size.x, size.y, size.z, 1);
    const distance =
      ((maximum / (2 * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)))) *
        1.55) /
      Math.min(camera.aspect, 1);
    controls.target.set(0, 0, size.z * 0.35);
    camera.position.set(
      distance * 0.8,
      -distance * 0.8,
      distance * 0.65 + size.z * 0.35,
    );
    camera.near = Math.max(distance / 1000, 0.01);
    camera.far = distance * 100;
    camera.updateProjectionMatrix();
    controls.update();
  };
  const resize = new ResizeObserver(() => {
    camera.aspect =
      Math.max(mount.clientWidth, 1) / Math.max(mount.clientHeight, 1);
    camera.updateProjectionMatrix();
    renderer.setSize(mount.clientWidth, mount.clientHeight);
  });
  resize.observe(mount);
  const loader = new STLLoader();
  const requests = new LatestRequest();
  // The vendored JavaScript owns these GPU resources; keep them in this lifecycle.
  let mesh: any;
  const clear = () => {
    requests.invalidate();
    if (mesh) {
      scene.remove(mesh);
      mesh.geometry.dispose();
      mesh = undefined;
    }
  };
  const loadModel = async (url: string) => {
    clear();
    const request = requests.begin();
    let geometry: any;
    try {
      geometry = await loader.loadAsync(url);
      if (!request.isCurrent()) {
        geometry.dispose();
        return { kind: "stale" } as const;
      }
      geometry.computeBoundingBox();
      const box = geometry.boundingBox;
      if (!box || box.isEmpty()) throw new Error("Model has no geometry");
      box.getSize(size);
      if (!size.toArray().every(Number.isFinite))
        throw new Error("Invalid model dimensions");
      const center = box.getCenter(new THREE.Vector3());
      geometry.translate(-center.x, -center.y, -box.min.z);
      mesh = new THREE.Mesh(geometry, material);
      scene.add(mesh);
      fit();
      return {
        kind: "success",
        dimensions: `${size.x.toFixed(1)} × ${size.y.toFixed(1)} × ${size.z.toFixed(1)} mm`,
      } as const;
    } catch (error) {
      geometry?.dispose();
      if (!request.isCurrent()) return { kind: "stale" } as const;
      throw error;
    }
  };
  renderer.setAnimationLoop(() => renderer.render(scene, camera));
  return {
    loadModel,
    fit,
    zoom: (factor: number) => {
      camera.position
        .sub(controls.target)
        .multiplyScalar(factor)
        .add(controls.target);
      controls.update();
    },
    cancelLoad: () => requests.invalidate(),
    destroy: () => {
      clear();
      renderer.setAnimationLoop(null);
      resize.disconnect();
      observer.disconnect();
      scheme.removeEventListener("change", theme);
      controls.dispose();
      grid.geometry.dispose();
      grid.material.dispose();
      material.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      renderer.domElement.remove();
    },
  };
}
