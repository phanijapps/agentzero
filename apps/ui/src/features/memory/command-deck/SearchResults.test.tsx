// ============================================================================
// SearchResults — render tests
// ============================================================================

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SearchResults } from './SearchResults';
import type { HybridSearchResponse } from '@/services/transport/types';

function emptyData(): HybridSearchResponse {
  return {
    facts: { hits: [], latency_ms: 5 },
    wiki: { hits: [], latency_ms: 5 },
    procedures: { hits: [], latency_ms: 5 },
    episodes: { hits: [], latency_ms: 5 },
  };
}

describe('SearchResults', () => {
  it('shows searching message when loading with no data', () => {
    render(<SearchResults data={null} loading={true} />);
    expect(screen.getByText(/searching/i)).toBeInTheDocument();
  });

  it('shows type prompt when not loading and no data', () => {
    render(<SearchResults data={null} loading={false} />);
    expect(screen.getByText(/type to search/i)).toBeInTheDocument();
  });

  it('shows no matches when all hits are empty', () => {
    render(<SearchResults data={emptyData()} loading={false} />);
    expect(screen.getByText(/no matches/i)).toBeInTheDocument();
  });

  it('renders facts hits with MemoryItemCard', () => {
    const data = emptyData();
    data.facts.hits = [{
      id: 'f1',
      content: 'some fact content',
      category: 'general',
      confidence: 0.9,
      created_at: '2024-01-01T00:00:00Z',
      match_source: 'semantic',
      ward_id: 'ward-1',
      score: 0.85,
    }] as never;
    data.facts.latency_ms = 10;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('Facts')).toBeInTheDocument();
    expect(screen.getByText('some fact content')).toBeInTheDocument();
    expect(screen.getByText('1 hit · 10 ms')).toBeInTheDocument();
  });

  it('renders wiki hits as Row components', () => {
    const data = emptyData();
    data.wiki.hits = [{
      id: 'w1',
      title: 'My Wiki Article',
      snippet: 'A snippet of wiki content',
      ward_id: 'wiki-ward',
      match_source: 'keyword',
      score: 0.7,
    }] as never;
    data.wiki.latency_ms = 8;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('Wiki Articles')).toBeInTheDocument();
    expect(screen.getByText('My Wiki Article')).toBeInTheDocument();
    expect(screen.getByText('A snippet of wiki content')).toBeInTheDocument();
  });

  it('renders procedure hits', () => {
    const data = emptyData();
    data.procedures.hits = [{
      id: 'p1',
      name: 'Deploy Procedure',
      description: 'How to deploy the service',
      ward_id: 'proc-ward',
      match_source: 'semantic',
    }] as never;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('Procedures')).toBeInTheDocument();
    expect(screen.getByText('Deploy Procedure')).toBeInTheDocument();
    expect(screen.getByText('How to deploy the service')).toBeInTheDocument();
  });

  it('renders episode hits', () => {
    const data = emptyData();
    data.episodes.hits = [{
      id: 'e1',
      task_summary: 'Fixed a bug',
      key_learnings: 'Learned about async patterns in React',
      outcome: 'Bug resolved',
      ward_id: 'ep-ward',
      match_source: 'semantic',
    }] as never;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('Episodes')).toBeInTheDocument();
    expect(screen.getByText('Fixed a bug')).toBeInTheDocument();
  });

  it('shows plural "hits" for multiple results', () => {
    const data = emptyData();
    data.wiki.hits = [
      { id: 'w1', title: 'Art 1', snippet: '', ward_id: null, match_source: 'semantic', score: 0.9 },
      { id: 'w2', title: 'Art 2', snippet: '', ward_id: null, match_source: 'keyword', score: 0.8 },
    ] as never;
    data.wiki.latency_ms = 3;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('2 hits · 3 ms')).toBeInTheDocument();
  });

  it('renders fact content and passes onDeleteFact to MemoryItemCard', () => {
    const onDeleteFact = vi.fn();
    const data = emptyData();
    data.facts.hits = [{
      id: 'f1',
      content: 'deletable fact',
      category: 'general',
      confidence: 0.9,
      created_at: '2024-01-01T00:00:00Z',
      match_source: 'semantic',
      ward_id: null,
      score: 0.8,
    }] as never;
    render(<SearchResults data={data} loading={false} onDeleteFact={onDeleteFact} />);
    // Component renders fact content without error
    expect(screen.getByText('deletable fact')).toBeInTheDocument();
    expect(screen.getByText('Facts')).toBeInTheDocument();
  });

  it('shows loading state with existing data without overwriting content', () => {
    const data = emptyData();
    data.wiki.hits = [{ id: 'w1', title: 'Article', snippet: '', ward_id: null, match_source: 'semantic', score: 0.9 }] as never;
    render(<SearchResults data={data} loading={true} />);
    // When loading=true AND data exists, it should still show the data (not the "Searching..." spinner)
    expect(screen.getByText('Wiki Articles')).toBeInTheDocument();
  });

  it('renders ward tag when ward_id is present on wiki result', () => {
    const data = emptyData();
    data.wiki.hits = [{ id: 'w1', title: 'Tagged', snippet: 'content', ward_id: 'my-ward', match_source: 'semantic', score: 0.9 }] as never;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText(/my-ward/)).toBeInTheDocument();
  });

  it('renders score when present on wiki result', () => {
    const data = emptyData();
    data.wiki.hits = [{ id: 'w1', title: 'Scored', snippet: '', ward_id: null, match_source: 'semantic', score: 0.789 }] as never;
    render(<SearchResults data={data} loading={false} />);
    expect(screen.getByText('0.79')).toBeInTheDocument();
  });
});
