import { createApp } from "vue";
import { createPinia } from "pinia";
import router from "./router";
import App from "./App.vue";
import { useUiStore } from "./stores/ui";
import "./style.css";

const app = createApp(App);

app.use(createPinia());
app.use(router);

// 挂载前先把主题写到 <html>，避免首帧闪色
useUiStore();

app.mount("#app");
