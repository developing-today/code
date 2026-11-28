sections: 31-which-client-gets-asked, 4-mcpx-as-a-client, 41-server-initiated-requests-mcpx-answers, 6-what-is-still-missing
kind: Fixed
summary: protocol.md no longer asserts the pre-fix capability behaviour (#1930): the host gates read elicitation per mode and sampling per sub-capability (conn.go CanElicit/CanSampleTools/CanSampleContext), and roots/list is relayed to a declaring host or answered from configured daemon.roots, so the "coarser gates", "daemon passes none" and "declared with nothing answering" passages are corrected and the fixed gap leaves §6
