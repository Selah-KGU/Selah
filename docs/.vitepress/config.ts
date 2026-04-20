import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Selah',
  description: '新月の下で、知性を繋ぐ。すべての関学生に。',
  lang: 'ja',
  base: '/',
  head: [
    ['link', { rel: 'icon', href: '/logo.png' }],
  ],
  themeConfig: {
    logo: '/logo.png',
    nav: [
      { text: 'ホーム', link: '/' },
      { text: 'ガイド', link: '/guide/' },
      { text: '機能紹介', link: '/guide/features' },
      {
        text: 'v1.0.0',
        items: [
          { text: 'リリースノート', link: 'https://github.com/mirai-mamori/Selah/releases' },
        ],
      },
      {
        text: '法的情報',
        items: [
          { text: 'プライバシーポリシー', link: '/privacy' },
          { text: '利用規約', link: '/terms' },
        ],
      },
    ],
    sidebar: {
      '/guide/': [
        {
          text: 'はじめに',
          items: [
            { text: '概要', link: '/guide/' },
            { text: 'インストール', link: '/guide/install' },
          ],
        },
        {
          text: '機能',
          items: [
            { text: '機能一覧', link: '/guide/features' },
            { text: 'ホーム', link: '/guide/home' },
            { text: '時間割', link: '/guide/timetable' },
            { text: 'TODO / 課題', link: '/guide/todo' },
            { text: 'メール', link: '/guide/mail' },
            { text: '成績照会', link: '/guide/grades' },
            { text: 'シラバス検索', link: '/guide/syllabus' },
            { text: 'AI 機能', link: '/guide/ai' },
          ],
        },
        {
          text: 'その他',
          items: [
            { text: 'アーキテクチャ', link: '/guide/architecture' },
            { text: 'FAQ', link: '/guide/faq' },
          ],
        },
      ],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/mirai-mamori/Selah' },
    ],
    footer: {
      message: '<a href="/privacy">プライバシーポリシー</a> | <a href="/terms">利用規約</a> | MIT License',
      copyright: 'Copyright 2025-2026 mirai-mamori',
    },
    search: {
      provider: 'local',
    },
    outline: {
      label: '目次',
    },
    docFooter: {
      prev: '前のページ',
      next: '次のページ',
    },
  },
})
