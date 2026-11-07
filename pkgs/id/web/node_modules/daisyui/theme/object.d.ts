interface Theme {
  "color-scheme": string
  "--color-base-100": string
  "--color-base-200": string
  "--color-base-300": string
  "--color-base-content": string
  "--color-primary": string
  "--color-primary-content": string
  "--color-secondary": string
  "--color-secondary-content": string
  "--color-accent": string
  "--color-accent-content": string
  "--color-neutral": string
  "--color-neutral-content": string
  "--color-info": string
  "--color-info-content": string
  "--color-success": string
  "--color-success-content": string
  "--color-warning": string
  "--color-warning-content": string
  "--color-error": string
  "--color-error-content": string
  "--radius-selector": string
  "--radius-field": string
  "--radius-box": string
  "--size-selector": string
  "--size-field": string
  "--border": string
  "--depth": string
  "--noise": string
}


interface Themes {
  synthwave: Theme
  sunset: Theme
  dracula: Theme
  caramellatte: Theme
  fantasy: Theme
  lemonade: Theme
  business: Theme
  black: Theme
  night: Theme
  wireframe: Theme
  bumblebee: Theme
  cmyk: Theme
  pastel: Theme
  abyss: Theme
  forest: Theme
  nord: Theme
  cupcake: Theme
  emerald: Theme
  valentine: Theme
  corporate: Theme
  cyberpunk: Theme
  lofi: Theme
  garden: Theme
  acid: Theme
  aqua: Theme
  dim: Theme
  luxury: Theme
  winter: Theme
  halloween: Theme
  coffee: Theme
  autumn: Theme
  dark: Theme
  silk: Theme
  light: Theme
  retro: Theme
  [key: string]: Theme
}

declare const themes: Themes
export default themes