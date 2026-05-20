#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Claw 量化任务轨迹记录工具（Python版）
仅记录操作轨迹，不调用任何API，适配国内大模型+ccsswitch场景
"""
import os
import json
import time
import signal
import sys
from datetime import datetime

# ===================== 核心配置 =====================
# 轨迹保存目录（和原Claw保持一致，方便后续整理）
SESSIONS_DIR = os.path.join(".claude", "sessions")
# 全局会话数据
session = {
    "task": "",
    "start_timestamp": 0,
    "start_time": "",
    "end_timestamp": None,
    "end_time": None,
    "status": "recording",
    "mode": "only_record"  # 仅记录模式，无API调用
}

# ===================== 工具函数 =====================
def init_dir():
    """初始化轨迹保存目录"""
    if not os.path.exists(SESSIONS_DIR):
        try:
            os.makedirs(SESSIONS_DIR)
            print(f"✅ 初始化轨迹目录成功：{SESSIONS_DIR}")
        except Exception as e:
            print(f"⚠️  警告：创建目录失败（不影响使用）：{str(e)}")

def save_session():
    """保存/更新轨迹文件"""
    if session["start_timestamp"] == 0:
        return
    
    # 生成轨迹文件名（时间戳.json，和原Claw格式一致）
    filename = f"{session['start_timestamp']}.json"
    file_path = os.path.join(SESSIONS_DIR, filename)
    
    try:
        with open(file_path, "w", encoding="utf-8") as f:
            json.dump(session, f, ensure_ascii=False, indent=4)
        print(f"✅ 轨迹已保存至：{file_path}")
    except Exception as e:
        print(f"⚠️  警告：保存轨迹失败：{str(e)}")

def handle_exit(signal_num, frame):
    """处理Ctrl+C退出，优雅保存轨迹"""
    print("\n\n🛑 接收到退出信号，正在保存最终轨迹...")
    # 更新会话状态
    session["status"] = "completed"
    session["end_timestamp"] = int(time.time())
    session["end_time"] = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    # 保存最终轨迹
    save_session()
    print("✅ 轨迹记录完成！可在 .claude/sessions 查看文件")
    sys.exit(0)

# ===================== 主逻辑 =====================
def main():
    # 注册Ctrl+C信号处理
    signal.signal(signal.SIGINT, handle_exit)
    
    # 解析命令行参数
    if len(sys.argv) < 2 or sys.argv[1] in ["-h", "--help"]:
        print("="*50)
        print("Claw 量化任务轨迹记录工具（仅记录模式）")
        print("="*50)
        print("使用方法：")
        print("  python main.py \"你的量化任务指令\"")
        print("示例：")
        print("  python main.py \"检查E:\\code\\ETH策略.py的编码问题\"")
        print("  python main.py \"优化BTC回测脚本的性能\"")
        print("="*50)
        return
    
    # 初始化任务信息
    task = " ".join(sys.argv[1:])
    session["task"] = task
    session["start_timestamp"] = int(time.time())
    session["start_time"] = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    
    # 启动提示
    print("="*50)
    print("📝 Claw 仅记录轨迹模式已启动（Python版）")
    print(f"🎯 任务：{task}")
    print("💡 现在可在VS Code中完成任务，操作轨迹会记录")
    print("🔑 无需API密钥，无需调用任何大模型")
    print("🛑 按 Ctrl+C 结束记录并保存轨迹")
    print("="*50)
    
    # 初始化目录+保存初始轨迹
    init_dir()
    save_session()
    
    # 阻塞等待用户操作（无限循环，直到Ctrl+C）
    try:
        while True:
            time.sleep(1)
    except Exception as e:
        print(f"\n⚠️  程序异常：{str(e)}")
        handle_exit(None, None)

if __name__ == "__main__":
    main()