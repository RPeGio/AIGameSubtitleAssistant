import { createRouter, createWebHistory } from "vue-router";
import Welcome from "../views/Welcome.vue";
import Editor from "../views/Editor.vue";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "welcome",
      component: Welcome,
    },
    {
      path: "/editor/:path?",
      name: "editor",
      component: Editor,
    },
  ],
});

export default router;
