"""Framework integrations for the CRW SDK (extras-gated).

Each submodule requires its framework extra and is imported explicitly — the
base ``crw`` package stays dependency-free.

    pip install "crw[langchain]"   # then: from crw.integrations.langchain import CrwLoader
    pip install "crw[crewai]"      # then: from crw.integrations.crewai import CrwScrapeWebsiteTool

Nothing is imported here eagerly so that installing only the base package never
pulls a framework in.
"""
