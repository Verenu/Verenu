/// <reference types="vite/client" />

declare const __VERENU_GIT_SHA__: string;
declare const __VERENU_GIT_BRANCH__: string;
declare const __VERENU_BUILD_TIME__: string;
declare const __VERENU_GIT_DIRTY__: boolean;

declare module '*.css' {
  const content: Record<string, any>;
  export default content;
}

declare module '*.svg?raw' {
  const content: string;
  export default content;
}
