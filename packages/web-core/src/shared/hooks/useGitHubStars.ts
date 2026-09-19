import { useQuery } from '@tanstack/react-query';

async function fetchGitHubStars(): Promise<number | null> {
  try {
    const res = await fetch(
      'https://api.github.com/repos/BloopAI/vibe-kanban',
      { cache: 'no-store' }
    );

    if (!res.ok) {
      console.warn(`GitHub API error: ${res.status}`);
      return null;
    }

    const data = await res.json();
    if (typeof data?.stargazers_count === 'number') {
      return data.stargazers_count;
    }

    return null;
  } catch (error) {
    console.warn('Failed to fetch GitHub stars:', error);
    return null;
  }
}

/**
 * 个人版不显示 GitHub 徽标（设计文档 §8.1），调用方传 `enabled: false`
 * 连请求也不发。默认开启，团队版与云端构建行为不变。
 */
export function useGitHubStars(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ['github-stars'],
    queryFn: fetchGitHubStars,
    enabled: options.enabled ?? true,
    refetchInterval: 10 * 60 * 1000,
    staleTime: 10 * 60 * 1000,
    retry: false,
    refetchOnMount: false,
    placeholderData: (previousData) => previousData,
  });
}
