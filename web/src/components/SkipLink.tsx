/* 「跳到主内容」（#523）：键盘用户第一下 Tab 就能跳过顶栏、活动栏、侧栏。平时看不见，聚焦时出现在左上角。
   是页面内锚点（href="#main"），但不改地址栏：点了直接把焦点放进 <main id="main">，不触发路由、不发请求 */
export function SkipLink() {
  return (
    <a
      className="skip-link"
      href="#main"
      onClick={e => {
        const main = document.getElementById('main');
        if (!main) return;
        e.preventDefault();
        main.focus();
      }}
    >
      跳到主内容
    </a>
  );
}
