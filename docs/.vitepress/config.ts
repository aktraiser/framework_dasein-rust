import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Agentic-RS',
  description: 'High-performance Rust framework for building autonomous multi-agent AI systems',

  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }]
  ],

  themeConfig: {
    logo: '/logo.svg',

    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'API', link: '/api/' },
      { text: 'Examples', link: '/examples/' },
      {
        text: 'v0.1.0',
        items: [
          { text: 'Changelog', link: '/changelog' },
          { text: 'Contributing', link: '/contributing' }
        ]
      }
    ],

    sidebar: {
      '/guide/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What is Agentic-RS?', link: '/guide/' },
            { text: 'Getting Started', link: '/guide/getting-started' },
            { text: 'Architecture', link: '/guide/architecture' }
          ]
        },
        {
          text: 'Core Concepts',
          items: [
            { text: 'Agents', link: '/guide/agents' },
            { text: 'LLM Adapters', link: '/guide/llm-adapters' },
            { text: 'Sandboxes', link: '/guide/sandboxes' },
            { text: 'Message Bus', link: '/guide/message-bus' },
            { text: 'Storage', link: '/guide/storage' }
          ]
        },
        {
          text: 'Advanced',
          items: [
            { text: 'Orchestration', link: '/guide/orchestration' },
            { text: 'Workflows', link: '/guide/workflows' },
            { text: 'MCP Integration', link: '/guide/mcp' }
          ]
        }
      ],
      '/api/': [
        {
          text: 'Crates',
          items: [
            { text: 'agentic-core', link: '/api/core' },
            { text: 'agentic-llm', link: '/api/llm' },
            { text: 'agentic-sandbox', link: '/api/sandbox' },
            { text: 'agentic-bus', link: '/api/bus' },
            { text: 'agentic-storage', link: '/api/storage' },
            { text: 'agentic-orchestrator', link: '/api/orchestrator' },
            { text: 'agentic-mcp', link: '/api/mcp' }
          ]
        }
      ],
      '/examples/': [
        {
          text: 'Examples',
          items: [
            { text: 'Single Agent', link: '/examples/single-agent' },
            { text: 'Chat Console', link: '/examples/chat' },
            { text: 'Multi-Agent Workflow', link: '/examples/multi-agent' }
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/your-org/agentic-rs' }
    ],

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2024 Agentic-RS Contributors'
    },

    search: {
      provider: 'local'
    }
  }
})
