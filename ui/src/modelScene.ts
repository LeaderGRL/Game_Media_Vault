import {
  AmbientLight,
  Box3,
  DirectionalLight,
  Mesh,
  type Material,
  PerspectiveCamera,
  Scene,
  Texture,
  Vector3,
  WebGLRenderer,
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

export interface ModelSceneEvents {
  /** The model is loaded and drawn. */
  onReady: () => void;
  /** WebGL or the model is unavailable; nothing more is drawn. */
  onError: (reason: unknown) => void;
}

/** Side of the square the scene is drawn in when its host has no size yet. */
const FALLBACK_SIZE = 240;

/**
 * Draws the glTF model at `url` in `host`, turned by dragging, and returns what stops the
 * scene and frees what it holds. The scene is drawn only when the view changes.
 */
export function showModel(host: HTMLElement, url: string, events: ModelSceneEvents): () => void {
  let renderer: WebGLRenderer;
  try {
    renderer = new WebGLRenderer({ antialias: true, alpha: true });
  } catch (reason) {
    events.onError(reason);
    return () => {};
  }
  const width = host.clientWidth || FALLBACK_SIZE;
  const height = host.clientHeight || FALLBACK_SIZE;
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(width, height);
  host.appendChild(renderer.domElement);

  const scene = new Scene();
  scene.add(new AmbientLight(0xffffff, 1.6));
  const light = new DirectionalLight(0xffffff, 1.4);
  light.position.set(2, 3, 4);
  scene.add(light);
  const camera = new PerspectiveCamera(35, width / height, 0.01, 100);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enablePan = false;
  const draw = () => renderer.render(scene, camera);
  controls.addEventListener("change", draw);

  let stopped = false;
  new GLTFLoader().load(
    url,
    (gltf) => {
      if (stopped) {
        return;
      }
      scene.add(gltf.scene);
      // Three-quarters on from the left, so the front and the spine both show.
      const bounds = new Box3().setFromObject(gltf.scene);
      const center = bounds.getCenter(new Vector3());
      const distance = bounds.getSize(new Vector3()).length() * 2.2;
      camera.position
        .copy(center)
        .add(new Vector3(-0.6, 0.35, 1).normalize().multiplyScalar(distance));
      controls.target.copy(center);
      controls.update();
      draw();
      events.onReady();
    },
    undefined,
    (reason) => {
      if (!stopped) {
        events.onError(reason);
      }
    },
  );

  return () => {
    stopped = true;
    controls.removeEventListener("change", draw);
    controls.dispose();
    scene.traverse((object) => {
      if (object instanceof Mesh) {
        object.geometry.dispose();
        const materials: Material[] = Array.isArray(object.material)
          ? object.material
          : [object.material];
        for (const material of materials) {
          for (const value of Object.values(material)) {
            if (value instanceof Texture) {
              value.dispose();
            }
          }
          material.dispose();
        }
      }
    });
    renderer.dispose();
    renderer.domElement.remove();
  };
}
