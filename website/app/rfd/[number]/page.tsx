'use client'

import { use, useState } from 'react'
import Link from 'next/link'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Navbar } from '@/components/landing/Navbar'
import { getRFD, getRFDs, getStatusColor } from '@/lib/rfd-data'
import {
  Calendar,
  Users,
  MessageSquare,
  ExternalLink,
  Hash,
  Search,
  Filter,
  Send,
  User
} from 'lucide-react'
import { notFound } from 'next/navigation'

export default function RFDDetailPage({ params }: { params: Promise<{ number: string }> }) {
  const { number } = use(params)
  const rfdNumber = parseInt(number)
  const rfd = getRFD(rfdNumber)
  const allRFDs = getRFDs()

  const [searchQuery, setSearchQuery] = useState('')
  const [statusFilter, setStatusFilter] = useState<string>('all')
  const [comment, setComment] = useState('')

  if (!rfd) {
    notFound()
  }

  // Filter RFDs based on search and status
  const filteredRFDs = allRFDs.filter(r => {
    const matchesSearch = r.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
                         r.number.toString().includes(searchQuery)
    const matchesStatus = statusFilter === 'all' || r.status === statusFilter
    return matchesSearch && matchesStatus
  })

  return (
    <div className="min-h-screen bg-background text-foreground">
      <Navbar />

      {/* Two-Panel Layout */}
      <div className="flex h-[calc(100vh-5rem)]">
        {/* Left Sidebar - RFD List */}
        <aside className="w-80 border-r border-border bg-secondary/20 flex flex-col">
          {/* Header */}
          <div className="p-4 border-b border-border">
            <h2 className="text-lg font-bold text-foreground mb-3">All RFDs</h2>

            {/* Search */}
            <div className="relative mb-3">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
              <Input
                type="text"
                placeholder="Search RFDs..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-9 bg-background border-border"
              />
            </div>

            {/* Status Filter */}
            <div className="flex items-center gap-2">
              <Filter className="w-4 h-4 text-muted-foreground" />
              <select
                value={statusFilter}
                onChange={(e) => setStatusFilter(e.target.value)}
                className="flex-1 px-3 py-1.5 text-sm bg-background border border-border rounded-lg text-foreground"
              >
                <option value="all">All Statuses</option>
                <option value="prediscussion">Prediscussion</option>
                <option value="ideation">Ideation</option>
                <option value="discussion">Discussion</option>
                <option value="published">Published</option>
                <option value="committed">Committed</option>
                <option value="abandoned">Abandoned</option>
              </select>
            </div>
          </div>

          {/* RFD List - Scrollable */}
          <div className="flex-1 overflow-y-auto">
            <div className="p-2 space-y-1">
              {filteredRFDs.map((r) => (
                <Link
                  key={r.number}
                  href={`/rfd/${r.number}`}
                  className={`block p-3 rounded-lg transition-colors ${
                    r.number === rfdNumber
                      ? 'bg-primary/10 border border-primary/30'
                      : 'hover:bg-secondary/50'
                  }`}
                >
                  <div className="flex items-start gap-3">
                    <span className={`text-sm font-bold ${
                      r.number === rfdNumber ? 'text-primary' : 'text-muted-foreground'
                    }`}>
                      #{r.number}
                    </span>
                    <div className="flex-1 min-w-0">
                      <h3 className={`text-sm font-semibold mb-1 line-clamp-2 ${
                        r.number === rfdNumber ? 'text-primary' : 'text-foreground'
                      }`}>
                        {r.title}
                      </h3>
                      <div className="flex items-center gap-2">
                        <span className={`text-xs px-2 py-0.5 rounded-full capitalize ${getStatusColor(r.status)}`}>
                          {r.status}
                        </span>
                      </div>
                    </div>
                  </div>
                </Link>
              ))}
            </div>

            {filteredRFDs.length === 0 && (
              <div className="text-center py-12 text-muted-foreground text-sm">
                No RFDs match your filters
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="p-4 border-t border-border">
            <Link href="/rfd">
              <Button variant="outline" className="w-full border-border text-sm">
                View RFD Index
              </Button>
            </Link>
          </div>
        </aside>

        {/* Right Content Area - RFD Document */}
        <main className="flex-1 overflow-y-auto">
          <article className="max-w-4xl mx-auto px-8 py-12">
            {/* RFD Header */}
            <div className="mb-8">
              <div className="flex items-center gap-3 mb-4">
                <div className="flex items-center gap-2 text-2xl font-bold text-primary">
                  <Hash className="w-6 h-6" />
                  <span>{rfd.number}</span>
                </div>
                <span className={`text-sm px-3 py-1 rounded-full capitalize ${getStatusColor(rfd.status)}`}>
                  {rfd.status}
                </span>
              </div>

              <h1 className="text-4xl font-bold text-foreground mb-6">
                {rfd.title}
              </h1>

              {/* Tags */}
              <div className="flex flex-wrap gap-2 mb-6">
                {rfd.tags.map((tag) => (
                  <span key={tag} className="text-xs px-3 py-1 bg-secondary rounded-full text-secondary-foreground">
                    {tag}
                  </span>
                ))}
              </div>

              {/* Meta */}
              <Card className="bg-card border-border">
                <CardContent className="pt-4">
                  <div className="grid md:grid-cols-3 gap-4 text-sm">
                    <div>
                      <p className="text-muted-foreground mb-1">Authors</p>
                      <div className="flex items-center gap-2">
                        <Users className="w-4 h-4 text-primary" />
                        <span className="font-semibold text-foreground">{rfd.authors.join(', ')}</span>
                      </div>
                    </div>
                    <div>
                      <p className="text-muted-foreground mb-1">Created</p>
                      <div className="flex items-center gap-2">
                        <Calendar className="w-4 h-4 text-primary" />
                        <span className="font-semibold text-foreground">
                          {new Date(rfd.created).toLocaleDateString('en-US', {
                            month: 'short',
                            day: 'numeric',
                            year: 'numeric'
                          })}
                        </span>
                      </div>
                    </div>
                    <div>
                      <p className="text-muted-foreground mb-1">Updated</p>
                      <div className="flex items-center gap-2">
                        <Calendar className="w-4 h-4 text-primary" />
                        <span className="font-semibold text-foreground">
                          {new Date(rfd.updated).toLocaleDateString('en-US', {
                            month: 'short',
                            day: 'numeric',
                            year: 'numeric'
                          })}
                        </span>
                      </div>
                    </div>
                  </div>
                </CardContent>
              </Card>
            </div>

            {/* Document Content */}
            <div className="prose prose-lg prose-invert max-w-none mb-12">
              <div
                className="text-muted-foreground leading-relaxed"
                dangerouslySetInnerHTML={{
                  __html: rfd.content
                    .replace(/^# (.*$)/gim, '<h1 class="text-3xl font-bold text-foreground mt-8 mb-4">$1</h1>')
                    .replace(/^## (.*$)/gim, '<h2 class="text-2xl font-bold text-foreground mt-6 mb-3">$2</h2>')
                    .replace(/^### (.*$)/gim, '<h3 class="text-xl font-bold text-foreground mt-4 mb-2">$3</h3>')
                    .replace(/\*\*(.*?)\*\*/g, '<strong class="font-bold text-foreground">$1</strong>')
                    .replace(/\`\`\`(\w+)?\n([\s\S]*?)\`\`\`/g, '<pre class="bg-secondary/50 border border-border rounded-lg p-4 overflow-x-auto my-4"><code class="text-sm text-foreground">$2</code></pre>')
                    .replace(/\`([^`]+)\`/g, '<code class="bg-secondary px-1.5 py-0.5 rounded text-sm text-primary">$1</code>')
                    .replace(/^- (.*$)/gim, '<li class="ml-6 mb-2 text-muted-foreground">$1</li>')
                    .replace(/^(\d+)\. (.*$)/gim, '<li class="ml-6 mb-2 text-muted-foreground">$2</li>')
                    .split('\n\n')
                    .map(para => {
                      if (para.startsWith('<h') || para.startsWith('<li') || para.startsWith('<pre') || para.startsWith('<code')) {
                        return para
                      }
                      return `<p class="mb-4 text-muted-foreground">${para}</p>`
                    })
                    .join('\n')
                }}
              />
            </div>

            {/* GitHub Discussion Link */}
            <Card className="bg-primary/5 border-primary/30 mb-8">
              <CardContent className="pt-6 text-center">
                <h3 className="text-xl font-bold text-foreground mb-2">
                  Join the Discussion
                </h3>
                <p className="text-muted-foreground mb-4 text-sm">
                  The full discussion for this RFD happens on GitHub
                </p>
                <a href={rfd.discussion} target="_blank" rel="noopener noreferrer">
                  <Button className="bg-primary hover:bg-primary/90 text-primary-foreground">
                    <MessageSquare className="w-4 h-4 mr-2" />
                    Open GitHub Discussion
                    <ExternalLink className="w-4 h-4 ml-2" />
                  </Button>
                </a>
              </CardContent>
            </Card>

            {/* Comments Section (If Logged In) */}
            {user ? (
              <div className="border-t border-border pt-8">
                <h3 className="text-2xl font-bold text-foreground mb-6">Comments</h3>

                {/* Comment Form */}
                <Card className="bg-card border-border mb-6">
                  <CardContent className="pt-6">
                    <div className="flex items-start gap-3 mb-4">
                      <div className="w-10 h-10 bg-primary/10 rounded-full flex items-center justify-center flex-shrink-0">
                        <User className="w-5 h-5 text-primary" />
                      </div>
                      <div className="flex-1">
                        <p className="font-semibold text-foreground mb-1">guest</p>
                        <p className="text-xs text-muted-foreground">Leave a comment</p>
                      </div>
                    </div>

                    <textarea
                      value={comment}
                      onChange={(e) => setComment(e.target.value)}
                      placeholder="Share your thoughts on this RFD..."
                      className="w-full px-4 py-3 bg-background border border-border rounded-lg text-foreground resize-none focus:outline-none focus:border-primary"
                      rows={4}
                    />

                    <div className="flex justify-end mt-3">
                      <Button className="bg-primary hover:bg-primary/90 text-primary-foreground">
                        <Send className="w-4 h-4 mr-2" />
                        Post Comment
                      </Button>
                    </div>
                  </CardContent>
                </Card>

                {/* Sample Comments */}
                <div className="space-y-4">
                  <Card className="bg-card border-border">
                    <CardContent className="pt-6">
                      <div className="flex items-start gap-3 mb-3">
                        <div className="w-10 h-10 bg-primary/10 rounded-full flex items-center justify-center flex-shrink-0">
                          <User className="w-5 h-5 text-primary" />
                        </div>
                        <div className="flex-1">
                          <div className="flex items-center gap-2 mb-1">
                            <p className="font-semibold text-foreground">jordan_kim</p>
                            <span className="text-xs text-muted-foreground">2 days ago</span>
                          </div>
                          <p className="text-muted-foreground text-sm">
                            Great proposal! I think the PBFT approach makes sense for our use case. Have we considered the impact on block time?
                          </p>
                        </div>
                      </div>
                    </CardContent>
                  </Card>

                  <Card className="bg-card border-border">
                    <CardContent className="pt-6">
                      <div className="flex items-start gap-3 mb-3">
                        <div className="w-10 h-10 bg-primary/10 rounded-full flex items-center justify-center flex-shrink-0">
                          <User className="w-5 h-5 text-primary" />
                        </div>
                        <div className="flex-1">
                          <div className="flex items-center gap-2 mb-1">
                            <p className="font-semibold text-foreground">sarah_chen</p>
                            <span className="text-xs text-muted-foreground">1 day ago</span>
                          </div>
                          <p className="text-muted-foreground text-sm">
                            @jordan_kim Good point. The estimated increase from ~6s to ~10s seems reasonable for the added security guarantees.
                          </p>
                        </div>
                      </div>
                    </CardContent>
                  </Card>
                </div>
              </div>
            ) : (
              <Card className="bg-secondary/30 border-border">
                <CardContent className="pt-6 text-center">
                  <p className="text-muted-foreground mb-4">
                    Sign in to leave comments on this RFD
                  </p>
                  <Link href="/signin">
                    <Button variant="outline" className="border-border">
                      Sign In
                    </Button>
                  </Link>
                </CardContent>
              </Card>
            )}
          </article>
        </main>
      </div>
    </div>
  )
}
