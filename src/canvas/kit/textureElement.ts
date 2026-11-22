import { KitElement } from './registry';
import { textureOf, watchTextures } from './texture';

/** Changes only presentation attributes, retaining focused controls and tables. */
export abstract class TextureElement extends KitElement {
  private _stopTexture?: () => void;
  private _texture = () => this.setAttribute('data-hc-texture', textureOf(this));
  connectedCallback(): void {
    this._stopTexture?.();
    this._stopTexture = watchTextures(this._texture);
    this._texture();
    super.connectedCallback();
  }
  disconnectedCallback(): void { this._stopTexture?.(); this._stopTexture = undefined; }
}
