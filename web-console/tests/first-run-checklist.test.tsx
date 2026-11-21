import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FirstRunChecklist } from "../src/features/configuration/first-run-checklist.js";

describe("FirstRunChecklist", () => {
  it("未通过预检时默认展开并列出必备项", () => {
    render(<FirstRunChecklist invalidCount={3} onSelectGroup={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "首次配置检查单" })).toBeInTheDocument();
    expect(screen.getByText("按顺序补齐以下必备项后，本地预检才会通过。")).toBeInTheDocument();
    expect(screen.getByText("配置模型凭证")).toBeInTheDocument();
    expect(screen.getByText("补全一个平台入口")).toBeInTheDocument();
    expect(screen.getByText("核对文本模型路由")).toBeInTheDocument();
    expect(screen.getByText("保存并校验")).toBeInTheDocument();
    expect(screen.getByText("重启并确认")).toBeInTheDocument();
  });

  it("通过预检后默认折叠，展开后仍可跳转业务域", async () => {
    const onSelectGroup = vi.fn();
    render(<FirstRunChecklist invalidCount={0} onSelectGroup={onSelectGroup} />);

    expect(screen.getByText("当前已通过本地预检；仍可展开核对部署所需配置。")).toBeInTheDocument();
    expect(screen.queryByText("配置模型凭证")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "展开检查单" }));
    expect(screen.getByText("配置模型凭证")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "去平台接入" }));
    expect(onSelectGroup).toHaveBeenCalledWith("platforms");

    await userEvent.click(screen.getByRole("button", { name: "收起检查单" }));
    expect(screen.queryByText("配置模型凭证")).not.toBeInTheDocument();
  });
});
