"""Agent executor for WebSocket queries with streaming support."""

from __future__ import annotations

import atexit
import asyncio
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

from opendev.web.state import WebState
from opendev.web.logging_config import logger
from opendev.models.message import ChatMessage, Role
from opendev.models.agent_deps import AgentDependencies
from opendev.core.runtime import ConfigManager
from opendev.models.config import AppConfig


class AgentExecutor:
    """Executes agent queries in background with WebSocket streaming."""

    def __init__(self, state: WebState):
        """Initialize agent executor.

        Args:
            state: Shared web state
        """
        self.state = state
        self.executor = ThreadPoolExecutor(max_workers=4)
        atexit.register(self.executor.shutdown, wait=False)

        # Shared thread pool for parallel tool execution across sessions
        self._shared_parallel_executor = ThreadPoolExecutor(
            max_workers=5, thread_name_prefix="web-tool"
        )
        atexit.register(self._shared_parallel_executor.shutdown, wait=False)

        # Current ReactExecutor per session (for interrupt bridging)
        self._current_react_executors: Dict[str, Any] = {}

    def interrupt_session(self, session_id: str) -> bool:
        """Interrupt a running session's ReactExecutor.

        Args:
            session_id: Session ID to interrupt

        Returns:
            True if interrupt was requested, False if no executor found
        """
        executor = self._current_react_executors.get(session_id)
        if executor:
            return executor.request_interrupt()
        return False

    async def execute_query(
        self,
        message: str,
        ws_manager: Any,
        *,
        session_id: str,
        session: Any,
    ) -> None:
        """Execute query and stream results via WebSocket.

        Args:
            message: User query
            ws_manager: WebSocket manager for broadcasting
            session_id: Session ID for scoping this execution
            session: Pre-loaded Session object (avoids mutating current_session)
        """
        try:
            # Mark session as running
            self.state.set_session_running(session_id)
            await ws_manager.broadcast({
                "type": "session_activity",
                "data": {"session_id": session_id, "status": "running"},
            })

            # Broadcast message start
            try:
                await ws_manager.broadcast(
                    {
                        "type": "message_start",
                        "data": {
                            "messageId": str(time.time()),
                            "session_id": session_id,
                        },
                    }
                )
            except Exception as e:
                logger.error(f"Failed to broadcast message_start: {e}")

            # Run agent in thread pool to avoid blocking event loop
            loop = asyncio.get_event_loop()
            response = await loop.run_in_executor(
                self.executor,
                self._run_agent_sync,
                message,
                ws_manager,
                loop,
                session_id,
                session,
            )

            # ReactExecutor handles step-by-step persistence — just log the result
            logger.info(
                f"Agent response: summary={(response.get('summary') or '')[:100]}, "
                f"error={response.get('error')}"
            )

            # Broadcast message complete
            try:
                await ws_manager.broadcast(
                    {
                        "type": "message_complete",
                        "data": {
                            "messageId": str(time.time()),
                            "session_id": session_id,
                        },
                    }
                )
            except Exception as e:
                logger.error(f"Failed to broadcast message_complete: {e}")

        except Exception as e:
            # Broadcast error
            logger.error(f"Agent execution error: {e}")
            import traceback

            logger.error(traceback.format_exc())
            try:
                await ws_manager.broadcast({
                    "type": "error",
                    "data": {"message": str(e), "session_id": session_id},
                })
            except Exception as broadcast_err:
                logger.error(f"Failed to broadcast error: {broadcast_err}")
        finally:
            # Clean up ReactExecutor reference
            self._current_react_executors.pop(session_id, None)
            # Always mark session as idle and clean up injection queue
            self.state.set_session_idle(session_id)
            self.state.clear_injection_queue(session_id)
            try:
                await ws_manager.broadcast({
                    "type": "session_activity",
                    "data": {"session_id": session_id, "status": "idle"},
                })
            except Exception:
                pass

    def _run_agent_sync(
        self,
        message: str,
        ws_manager: Any,
        loop: asyncio.AbstractEventLoop,
        session_id: str,
        session: Any,
    ) -> Dict[str, Any]:
        """Run agent synchronously in thread pool using ReactExecutor.

        Args:
            message: User query
            ws_manager: WebSocket manager
            loop: Event loop for async operations
            session_id: Session ID for scoping
            session: Pre-loaded Session object

        Returns:
            Dict with summary, error, latency_ms
        """
        from opendev.core.runtime.services import RuntimeService
        from opendev.core.context_engineering.tools.implementations import (
            FileOperations,
            WriteTool,
            EditTool,
            BashTool,
            WebFetchTool,
            OpenBrowserTool,
            WebScreenshotTool,
        )
        from opendev.core.context_engineering.tools.implementations.web_search_tool import (
            WebSearchTool,
        )
        from opendev.core.context_engineering.tools.implementations.notebook_edit_tool import (
            NotebookEditTool,
        )
        from opendev.core.context_engineering.tools.implementations.ask_user_tool import AskUserTool
        from opendev.web.web_approval_manager import WebApprovalManager
        from opendev.web.web_ask_user_manager import WebAskUserManager
        from opendev.web.web_ui_callback import WebUICallback
        from opendev.web.ws_tool_broadcaster import WebSocketToolBroadcaster
        from opendev.repl.react_executor import ReactExecutor

        # Clear any previous interrupt flags
        self.state.clear_interrupt()

        # Resolve config/working directory from session (no mutation of current_session)
        config_manager, config, working_dir = self._resolve_runtime_context_for_session(session)

        # Initialize tools
        file_ops = FileOperations(config, working_dir)
        write_tool = WriteTool(config, working_dir)
        edit_tool = EditTool(config, working_dir)
        bash_tool = BashTool(config, working_dir)
        web_fetch_tool = WebFetchTool(config, working_dir)
        web_search_tool = WebSearchTool(config, working_dir)
        notebook_edit_tool = NotebookEditTool(working_dir)
        # Create web-based ask-user manager with session_id
        web_ask_user_manager = WebAskUserManager(ws_manager, loop, session_id=session_id)
        ask_user_tool = AskUserTool(ui_prompt_callback=web_ask_user_manager.prompt_user)
        open_browser_tool = OpenBrowserTool(config, working_dir)
        web_screenshot_tool = WebScreenshotTool(config, working_dir)

        # Create web-based approval manager with session_id
        web_approval_manager = WebApprovalManager(ws_manager, loop, session_id=session_id)

        # Create web UI callback for plan approval, subagent events, etc.
        web_ui_callback = WebUICallback(ws_manager, loop, session_id, self.state)

        # Build runtime suite
        runtime_service = RuntimeService(config_manager, self.state.mode_manager)
        runtime_suite = runtime_service.build_suite(
            file_ops=file_ops,
            write_tool=write_tool,
            edit_tool=edit_tool,
            bash_tool=bash_tool,
            web_fetch_tool=web_fetch_tool,
            web_search_tool=web_search_tool,
            notebook_edit_tool=notebook_edit_tool,
            ask_user_tool=ask_user_tool,
            open_browser_tool=open_browser_tool,
            web_screenshot_tool=web_screenshot_tool,
            mcp_manager=self.state.mcp_manager,
        )

        # Wire hooks system
        hook_manager = None
        try:
            from opendev.core.hooks.loader import load_hooks_config
            from opendev.core.hooks.manager import HookManager

            hooks_config = load_hooks_config(working_dir)
            if hooks_config and hooks_config.hooks:
                hook_manager = HookManager(
                    hooks_config, session_id=session_id, cwd=str(working_dir)
                )
                runtime_suite.tool_registry.set_hook_manager(hook_manager)
                subagent_mgr = runtime_suite.tool_registry.get_subagent_manager()
                if subagent_mgr and hasattr(subagent_mgr, "set_hook_manager"):
                    subagent_mgr.set_hook_manager(hook_manager)
        except Exception as e:
            logger.warning(f"Failed to wire hooks: {e}")

        # Set thinking level from web state
        from opendev.core.context_engineering.tools.handlers.thinking_handler import ThinkingLevel
        thinking_level_str = self.state.get_thinking_level()
        try:
            thinking_level = ThinkingLevel(thinking_level_str)
        except ValueError:
            thinking_level = ThinkingLevel.MEDIUM
        runtime_suite.tool_registry.thinking_handler.set_level(thinking_level)

        # Wrap tool registry with WebSocket broadcaster (includes session_id)
        wrapped_registry = WebSocketToolBroadcaster(
            runtime_suite.tool_registry,
            ws_manager,
            loop,
            working_dir=working_dir,
            session_id=session_id,
        )

        # Instantiate CostTracker for this execution
        from opendev.core.runtime.cost_tracker import CostTracker

        cost_tracker = CostTracker()

        # Get agent
        agent = runtime_suite.agents.normal
        agent.tool_registry = wrapped_registry
        agent._cost_tracker = cost_tracker

        # Point session manager at the right session for this execution
        self.state.session_manager.current_session = session

        # Prepare messages for the ReAct loop
        message_history = session.to_api_messages()

        # Inject system prompt (TUI path does this via query_enhancer.prepare_messages)
        if not message_history or message_history[0].get("role") != "system":
            message_history.insert(0, {"role": "system", "content": agent.system_prompt})

        # Create ReactExecutor (no console, no llm_caller, no tool_executor — Web UI mode)
        react_executor = ReactExecutor(
            session_manager=self.state.session_manager,
            config=config,
            mode_manager=self.state.mode_manager,
            console=None,
            llm_caller=None,
            tool_executor=None,
            cost_tracker=cost_tracker,
            parallel_executor=self._shared_parallel_executor,
        )

        # Wire hooks
        if hook_manager:
            react_executor.set_hook_manager(hook_manager)

        # Wire injection queue for mid-execution user messages
        react_executor._injection_queue = self.state.get_injection_queue(session_id)

        # Store for interrupt bridging
        self._current_react_executors[session_id] = react_executor

        # Execute unified ReAct loop
        try:
            summary, error, latency_ms = react_executor.execute(
                query=message,
                messages=message_history,
                agent=agent,
                tool_registry=wrapped_registry,
                approval_manager=web_approval_manager,
                undo_manager=self.state.undo_manager,
                ui_callback=web_ui_callback,
            )
            return {"summary": summary, "error": error, "latency_ms": latency_ms}
        except Exception as e:
            logger.error(f"ReactExecutor error: {e}")
            import traceback
            logger.error(traceback.format_exc())
            return {"summary": None, "error": str(e), "latency_ms": 0}

    def _resolve_runtime_context_for_session(
        self, session: Any
    ) -> Tuple[ConfigManager, AppConfig, Path]:
        """Determine config manager, config, and working dir for a specific session."""
        if session and session.working_directory:
            working_dir = Path(session.working_directory).expanduser().resolve()
            config_manager = ConfigManager(working_dir)
            config = config_manager.get_config()
        else:
            config_manager = self.state.config_manager
            config = config_manager.get_config()
            working_dir = Path(config_manager.working_dir).resolve()

        try:
            config_manager.ensure_directories()
        except Exception:
            pass

        return config_manager, config, working_dir
