import { createApp } from "vue";
import { createPinia } from "pinia";
import router from "./router";
import App from "./App.vue";
import { useUiStore } from "./stores/ui";
import "./style.css";

const app = createApp(App);

app.use(createPinia());
app.use(router);

// 挂载前先把主题写到 <html>、把界面缩放推给 WebView，避免首帧闪色。
// 这两个副作用原先藏在 store 求值期（= 任何 import 都会触发），现已收进显式入口。
useUiStore().initUiShell();

app.mount("#app");
