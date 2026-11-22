/**
 * L0 环境画布与可选背景层：
 * 默认渲染昼夜自适应环境光晕（晨雾暖杏雾蓝 / 暮色深海暖琥珀），使 L1 玻璃外壳自然透出光影；
 * 启用特殊九宫格或自定义背景图时，对应层覆盖在 L0 之上（由 data-background 属性驱动）。
 */
export function ConsoleBackground() {
  return (
    <>
      <div className="console-background console-background--ambient" aria-hidden="true" />
      <div className="console-background console-background--special" aria-hidden="true">
        <div className="console-background-grid">
          {Array.from({ length: 9 }, (_, index) => (
            <div key={index} className="console-background-grid-cell" />
          ))}
        </div>
      </div>
      <div className="console-background console-background--custom" aria-hidden="true" />
    </>
  );
}
