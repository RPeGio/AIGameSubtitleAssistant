import { createRouter, createWebHistory } from "vue-router";
import Welcome from "../views/Welcome.vue";
import ProjectLayout from "../layouts/ProjectLayout.vue";
import CorpusView from "../views/CorpusView.vue";
import AsrView from "../views/AsrView.vue";
import FuseView from "../views/FuseView.vue";
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
      path: "/project/:path?",
      component: ProjectLayout,
      children: [
        { path: "", redirect: { name: "editor" } },
        { path: "corpus", name: "corpus", component: CorpusView },
        { path: "asr", name: "asr", component: AsrView },
        { path: "fuse", name: "fuse", component: FuseView },
        { path: "editor", name: "editor", component: Editor },
      ],
    },
    // 旧单页路由：/editor/:path → 新工作流布局的编辑页
    {
      path: "/editor/:path?",
      redirect: (to) => `/project/${to.params.path ?? ""}/editor`,
    },
  ],
});

export default router;
