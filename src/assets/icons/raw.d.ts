// Vite 的 ?raw 导入把文件内容作为字符串导出。
// 只用在本目录下的本地静态图标——不做任何远端内容内联。
declare module "*.svg?raw" {
  const content: string;
  export default content;
}
