"""The MCP transport exception must not authorize a second provider stack."""

import contextlib
import io
import unittest

from rig_boundary_check import check_direct_dependencies


class DependencyBoundaryTests(unittest.TestCase):
    def metadata(self, package, rename):
        return {"packages": [{"name": package, "dependencies": [{
            "name": "reqwest", "req": "^0.13", "rename": rename,
        }]}]}

    def test_mcp_sdk_transport_edge_is_allowed(self):
        check_direct_dependencies(self.metadata("agent-runtime", "reqwest-mcp"))

    def test_other_runtime_reqwest_edges_remain_forbidden(self):
        for rename in [None, "reqwest-provider"]:
            with self.subTest(rename=rename), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    check_direct_dependencies(self.metadata("agent-runtime", rename))

    def test_mcp_alias_outside_runtime_remains_forbidden(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            check_direct_dependencies(self.metadata("gateway-execution", "reqwest-mcp"))


if __name__ == "__main__":
    unittest.main()
